use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use eframe::egui;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

struct MusicPlayer {
    sink: Option<Sink>,
    _stream: Option<OutputStream>,
    _stream_handle: Option<OutputStreamHandle>,
    current_track: Option<PathBuf>,
    current_position: Arc<Mutex<Duration>>,
    total_duration: Option<Duration>,
    playback_start_time: Option<Instant>,
    accumulated_time: Duration,  // Track accumulated time during pauses
    is_playing: bool,
    volume: f32,
}

impl Default for MusicPlayer {
    fn default() -> Self {
        let (stream, stream_handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&stream_handle).unwrap();
        sink.set_volume(0.5);

        Self {
            sink: Some(sink),
            _stream: Some(stream),
            _stream_handle: Some(stream_handle),
            current_track: None,
            current_position: Arc::new(Mutex::new(Duration::from_secs(0))),
            total_duration: None,
            playback_start_time: None,
            accumulated_time: Duration::from_secs(0),
            is_playing: false,
            volume: 0.5,
        }
    }
}

impl eframe::App for MusicPlayer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request continuous repainting to update the UI
        ctx.request_repaint();

        // Update current position if playing
        if self.is_playing {
            if let Some(start_time) = self.playback_start_time {
                let current_segment_time = start_time.elapsed();
                let total_elapsed = self.accumulated_time + current_segment_time;

                if let Ok(mut pos) = self.current_position.lock() {
                    *pos = total_elapsed;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Music Player");

            // Current track display
            if let Some(path) = &self.current_track {
                ui.label(format!("Now playing: {}", path.file_name().unwrap().to_string_lossy()));
            } else {
                ui.label("No track selected");
            }

            // Player controls
            ui.horizontal(|ui| {
                if ui.button("Open").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Audio", &["mp3", "wav", "flac", "ogg"])
                        .pick_file()
                    {
                        self.load_track(path);
                    }
                }

                if self.is_playing {
                    if ui.button("⏸ Pause").clicked() {
                        if let Some(sink) = &self.sink {
                            sink.pause();
                            self.is_playing = false;

                            // Save accumulated time when pausing
                            if let Some(start_time) = self.playback_start_time {
                                self.accumulated_time += start_time.elapsed();
                                self.playback_start_time = None;
                            }
                        }
                    }
                } else {
                    if ui.button("▶ Play").clicked() {
                        if let Some(sink) = &self.sink {
                            sink.play();
                            self.is_playing = true;
                            // Start counting from now, but keep the accumulated time
                            self.playback_start_time = Some(Instant::now());
                        }
                    }
                }

                if ui.button("⏹ Stop").clicked() {
                    if let Some(sink) = &self.sink {
                        sink.stop();
                        self.is_playing = false;
                        self.playback_start_time = None;
                        self.accumulated_time = Duration::from_secs(0);

                        // Reset position
                        if let Ok(mut pos) = self.current_position.lock() {
                            *pos = Duration::from_secs(0);
                        }

                        // Recreate sink
                        if let Some(stream_handle) = &self._stream_handle {
                            self.sink = Some(Sink::try_new(stream_handle).unwrap());
                            self.sink.as_mut().unwrap().set_volume(self.volume);

                            // Clone the path if there is one
                            if let Some(path) = self.current_track.clone() {
                                self.load_file(&path);
                            }
                        }
                    }
                }
            });

            // Volume control
            ui.horizontal(|ui| {
                ui.label("Volume:");
                if ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0)).changed() {
                    if let Some(sink) = &self.sink {
                        sink.set_volume(self.volume);
                    }
                }
            });

            // Progress bar with actual progress
            if let Some(_) = &self.current_track {
                let progress = if let (Some(total), Ok(current)) = (self.total_duration, self.current_position.lock()) {
                    if total.as_secs() > 0 {
                        current.as_secs_f32() / total.as_secs_f32()
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                // Clamp progress to be between 0.0 and 1.0
                let progress = progress.max(0.0).min(1.0);

                // Display time
                let current_secs = if let Ok(current) = self.current_position.lock() {
                    current.as_secs()
                } else {
                    0
                };

                let total_secs = self.total_duration.map_or(0, |d| d.as_secs());

                ui.label(format!("{:02}:{:02} / {:02}:{:02}",
                                 current_secs / 60, current_secs % 60,
                                 total_secs / 60, total_secs % 60));

                ui.add(egui::ProgressBar::new(progress).show_percentage());
            }
        });
    }
}

impl MusicPlayer {
    fn load_track(&mut self, path: PathBuf) {
        if let Some(sink) = &self.sink {
            sink.stop();
        }

        // Get duration of track
        self.estimate_track_duration(&path);

        // Reset current position and accumulated time
        if let Ok(mut pos) = self.current_position.lock() {
            *pos = Duration::from_secs(0);
        }
        self.accumulated_time = Duration::from_secs(0);

        self.load_file(&path);
        self.current_track = Some(path);
        self.is_playing = true;
        self.playback_start_time = Some(Instant::now());
    }

    fn estimate_track_duration(&mut self, path: &Path) {
        // Try to get duration from the file
        if let Ok(file) = File::open(path) {
            let reader = BufReader::new(file);
            if let Ok(source) = Decoder::new(reader) {
                self.total_duration = Some(source.total_duration().unwrap_or(Duration::from_secs(0)));
            }
        }
    }

    fn load_file(&mut self, path: &Path) {
        if let Ok(file) = File::open(path) {
            let reader = BufReader::new(file);
            if let Ok(source) = Decoder::new(reader) {
                // Store the duration if available
                if self.total_duration.is_none() {
                    self.total_duration = source.total_duration();
                }

                if let Some(sink) = &self.sink {
                    sink.append(source);
                    sink.play();
                }
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Music Player",
        native_options,
        Box::new(|_cc| Ok(Box::new(MusicPlayer::default()))),
    )
}