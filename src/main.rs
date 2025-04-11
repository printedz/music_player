use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use rfd::FileDialog;

struct MusicPlayer {
    sink: Option<Sink>,
    _stream: Option<OutputStream>,
    _stream_handle: Option<OutputStreamHandle>,
    current_track: Option<PathBuf>,
    current_position: Arc<Mutex<Duration>>,
    total_duration: Option<Duration>,
    playback_start_time: Option<Instant>,
    accumulated_time: Duration,
    is_playing: bool,
    volume: f32,
    // New field to track position during slider drag
    slider_position: Option<Duration>,
}

impl Default for MusicPlayer {
    fn default() -> Self {
        // Create audio stream
        let (stream, stream_handle) = OutputStream::try_default().unwrap();

        Self {
            sink: None,
            _stream: Some(stream),
            _stream_handle: Some(stream_handle),
            current_track: None,
            current_position: Arc::new(Mutex::new(Duration::from_secs(0))),
            total_duration: None,
            playback_start_time: None,
            accumulated_time: Duration::from_secs(0),
            is_playing: false,
            volume: 1.0,
            slider_position: None,
        }
    }
}

impl eframe::App for MusicPlayer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Music Player");

            // Update current position based on playback time
            if self.is_playing {
                if let Some(start_time) = self.playback_start_time {
                    let elapsed = start_time.elapsed();
                    let current_pos = self.accumulated_time + elapsed;
                    *self.current_position.lock().unwrap() = current_pos;

                    // Check if we've reached the end of the track
                    if let Some(total) = self.total_duration {
                        if current_pos >= total {
                            self.is_playing = false;
                            self.accumulated_time = Duration::from_secs(0);
                            *self.current_position.lock().unwrap() = Duration::from_secs(0);
                            self.playback_start_time = None;

                            if let Some(sink) = &self.sink {
                                sink.stop();
                            }
                        }
                    }
                }
            }

            // File selection button
            if ui.button("Open File").clicked() {
                if let Some(path) = FileDialog::new()
                    .add_filter("Audio", &["mp3", "wav", "ogg", "flac"])
                    .pick_file()
                {
                    self.load_track(path);
                }
            }

            ui.horizontal(|ui| {
                // Play/Pause button
                if self.current_track.is_some() {
                    let button_text = if self.is_playing { "Pause" } else { "Play" };
                    if ui.button(button_text).clicked() {
                        if self.is_playing {
                            // Pause playback
                            if let Some(sink) = &self.sink {
                                sink.pause();
                                if let Some(start_time) = self.playback_start_time {
                                    self.accumulated_time += start_time.elapsed();
                                    self.playback_start_time = None;
                                }
                                self.is_playing = false;
                            }
                        } else {
                            // Resume or start playback
                            if let Some(sink) = &self.sink {
                                if self.accumulated_time == Duration::from_secs(0) {
                                    // Fresh playback
                                    if let Some(path) = self.current_track.clone() {
                                        self.load_file(&path);
                                    }
                                } else {
                                    // Resume from paused position
                                    sink.play();
                                    self.playback_start_time = Some(Instant::now());
                                    self.is_playing = true;
                                }
                            }
                        }
                    }

                    // Stop button
                    if ui.button("Stop").clicked() {
                        if let Some(sink) = &self.sink {
                            sink.stop();
                        }
                        self.is_playing = false;
                        self.accumulated_time = Duration::from_secs(0);
                        *self.current_position.lock().unwrap() = Duration::from_secs(0);
                        self.playback_start_time = None;
                    }
                }
            });

            // Volume slider (modified to show 0% to 100%)
            ui.horizontal(|ui| {
                ui.label("Volume:");
                // Convert volume to percentage for display
                let mut volume_percent = self.volume * 100.0;
                if ui.add(egui::Slider::new(&mut volume_percent, 0.0..=100.0)
                    .suffix("%"))
                    .changed() {
                    // Convert percentage back to 0.0-1.0 range
                    self.volume = volume_percent / 100.0;
                    if let Some(sink) = &self.sink {
                        sink.set_volume(self.volume);
                    }
                }
            });

            // Show track info
            if let Some(path) = &self.current_track {
                ui.separator();

                let file_name = path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Unknown");

                ui.label(format!("Playing: {}", file_name));

                if let Some(total_duration) = self.total_duration {
                    // Get the current position to display
                    let display_position = if let Some(slider_pos) = self.slider_position {
                        // If slider is being dragged, show slider position
                        slider_pos
                    } else {
                        // Otherwise show actual playback position
                        *self.current_position.lock().unwrap()
                    };

                    // Format current position and total duration as MM:SS
                    let current_mins = display_position.as_secs() / 60;
                    let current_secs = display_position.as_secs() % 60;
                    let total_mins = total_duration.as_secs() / 60;
                    let total_secs = total_duration.as_secs() % 60;

                    ui.label(format!(
                        "{:02}:{:02} / {:02}:{:02}",
                        current_mins, current_secs, total_mins, total_secs
                    ));

                    // Playback slider
                    let total_secs = total_duration.as_secs_f32();

                    // Convert position to seconds for the slider
                    let mut current_secs = display_position.as_secs_f32();

                    let slider_response = ui.add(
                        egui::Slider::new(&mut current_secs, 0.0..=total_secs)
                            .show_value(false)
                            .trailing_fill(true)
                    );

                    // Update the displayed position during dragging without seeking
                    if slider_response.dragged() {
                        self.slider_position = Some(Duration::from_secs_f32(current_secs));
                    }

                    // Only seek in the file when the drag stops
                    if slider_response.drag_stopped() {
                        let new_position = Duration::from_secs_f32(current_secs);

                        // Clear the temporary slider position
                        self.slider_position = None;

                        // If playing, stop current playback and restart at new position
                        if let Some(track_path) = self.current_track.clone() {
                            if self.is_playing {
                                if let Some(sink) = &self.sink {
                                    sink.stop();
                                }
                                self.load_file_with_seek(&track_path, new_position);
                            } else {
                                // Just update the position if not playing
                                self.accumulated_time = new_position;
                                *self.current_position.lock().unwrap() = new_position;
                            }
                        }
                    }

                    // Handle clicks directly on the slider (not dragging)
                    if slider_response.clicked() && !slider_response.dragged() {
                        let new_position = Duration::from_secs_f32(current_secs);

                        // Clear the temporary slider position
                        self.slider_position = None;

                        // If playing, stop current playback and restart at new position
                        if let Some(track_path) = self.current_track.clone() {
                            if self.is_playing {
                                if let Some(sink) = &self.sink {
                                    sink.stop();
                                }
                                self.load_file_with_seek(&track_path, new_position);
                            } else {
                                // Just update the position if not playing
                                self.accumulated_time = new_position;
                                *self.current_position.lock().unwrap() = new_position;
                            }
                        }
                    }
                }
            }
        });

        // Request continuous redraw to update progress
        ctx.request_repaint();
    }
}

impl MusicPlayer {
    fn load_track(&mut self, path: PathBuf) {
        // Stop any current playback
        if let Some(sink) = &self.sink {
            sink.stop();
        }

        self.current_track = Some(path.clone());
        self.accumulated_time = Duration::from_secs(0);
        *self.current_position.lock().unwrap() = Duration::from_secs(0);
        self.playback_start_time = None;
        self.is_playing = false;
        self.slider_position = None;

        // Estimate track duration
        self.estimate_track_duration(&path);

        // Start playing the new track
        self.load_file(&path);
    }

    fn estimate_track_duration(&mut self, path: &Path) {
        // This is a simple estimation and might not be accurate for all formats
        // For more accurate duration, you'd need a dedicated audio metadata library
        if let Ok(file) = File::open(path) {
            let source = Decoder::new(BufReader::new(file)).ok();

            if let Some(source) = source {
                if let Some(duration) = source.total_duration() {
                    self.total_duration = Some(duration);
                    return;
                }
            }
        }

        // Fallback duration if we can't determine it
        self.total_duration = Some(Duration::from_secs(300)); // 5 minutes default
    }

    fn load_file(&mut self, path: &Path) {
        self.load_file_with_seek(path, Duration::from_secs(0));
    }

    fn load_file_with_seek(&mut self, path: &Path, position: Duration) {
        if let Ok(file) = File::open(path) {
            if let Ok(decoder) = Decoder::new(BufReader::new(file)) {
                if let Some(stream_handle) = &self._stream_handle {
                    // Initialize a new sink
                    if let Ok(sink) = Sink::try_new(stream_handle) {
                        // Set volume
                        sink.set_volume(self.volume);

                        // Skip to position if needed
                        if position > Duration::from_secs(0) {
                            let skipped_source = decoder.skip_duration(position);
                            sink.append(skipped_source);
                        } else {
                            sink.append(decoder);
                        }

                        // Start playback
                        sink.play();

                        self.sink = Some(sink);
                        self.is_playing = true;
                        self.accumulated_time = position;
                        self.playback_start_time = Some(Instant::now());
                    }
                }
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    let mut options = eframe::NativeOptions::default();
    options.viewport.inner_size = Some(egui::vec2(400.0, 175.0));

    eframe::run_native(
        "Music Player",
        options,
        Box::new(|_cc| Ok(Box::new(MusicPlayer::default()))),
    )
}