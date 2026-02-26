use daytona_toolbox_client::apis::computer_use_api;
use daytona_toolbox_client::apis::configuration::Configuration as ToolboxConfig;

use crate::client::convert_toolbox_error;
use crate::error::DaytonaError;

/// Service for computer use operations (mouse, keyboard, screenshot, display, recording).
pub struct ComputerUseService {
    pub(crate) config: ToolboxConfig,
}

impl ComputerUseService {
    /// Start the computer use service.
    pub async fn start(
        &self,
    ) -> Result<daytona_toolbox_client::models::ComputerUseStartResponse, DaytonaError> {
        let resp = computer_use_api::start_computer_use(&self.config)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }

    /// Stop the computer use service.
    pub async fn stop(
        &self,
    ) -> Result<daytona_toolbox_client::models::ComputerUseStopResponse, DaytonaError> {
        let resp = computer_use_api::stop_computer_use(&self.config)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }

    /// Get the status of the computer use service.
    pub async fn get_status(
        &self,
    ) -> Result<daytona_toolbox_client::models::ComputerUseStatusResponse, DaytonaError> {
        let status = computer_use_api::get_computer_use_status(&self.config)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(status)
    }

    /// Get the mouse sub-service.
    pub fn mouse(&self) -> MouseService<'_> {
        MouseService {
            config: &self.config,
        }
    }

    /// Get the keyboard sub-service.
    pub fn keyboard(&self) -> KeyboardService<'_> {
        KeyboardService {
            config: &self.config,
        }
    }

    /// Get the screenshot sub-service.
    pub fn screenshot(&self) -> ScreenshotService<'_> {
        ScreenshotService {
            config: &self.config,
        }
    }

    /// Get the display sub-service.
    pub fn display(&self) -> DisplayService<'_> {
        DisplayService {
            config: &self.config,
        }
    }

    /// Get the recording sub-service.
    pub fn recording(&self) -> RecordingService<'_> {
        RecordingService {
            config: &self.config,
        }
    }
}

/// Mouse operations.
pub struct MouseService<'a> {
    config: &'a ToolboxConfig,
}

impl MouseService<'_> {
    /// Get the current mouse position.
    pub async fn get_position(
        &self,
    ) -> Result<daytona_toolbox_client::models::MousePositionResponse, DaytonaError> {
        let pos = computer_use_api::get_mouse_position(self.config)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(pos)
    }

    /// Move the mouse to coordinates.
    pub async fn r#move(
        &self,
        x: i32,
        y: i32,
    ) -> Result<daytona_toolbox_client::models::MousePositionResponse, DaytonaError> {
        let req = daytona_toolbox_client::models::MouseMoveRequest {
            x: Some(x),
            y: Some(y),
        };
        let resp = computer_use_api::move_mouse(self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }

    /// Click at coordinates.
    ///
    /// The `button` parameter specifies which button ("left", "right", "middle").
    /// Set `double` to `true` for a double-click, matching Go/TypeScript SDK behavior.
    pub async fn click(
        &self,
        x: i32,
        y: i32,
        button: &str,
        double: Option<bool>,
    ) -> Result<daytona_toolbox_client::models::MouseClickResponse, DaytonaError> {
        let req = daytona_toolbox_client::models::MouseClickRequest {
            x: Some(x),
            y: Some(y),
            button: Some(button.to_string()),
            double,
        };
        let resp = computer_use_api::click(self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }

    /// Scroll at coordinates.
    pub async fn scroll(
        &self,
        x: i32,
        y: i32,
        _scroll_x: i32,
        scroll_y: i32,
    ) -> Result<daytona_toolbox_client::models::ScrollResponse, DaytonaError> {
        let direction = if scroll_y < 0 { "up" } else { "down" };
        let req = daytona_toolbox_client::models::MouseScrollRequest {
            x: Some(x),
            y: Some(y),
            amount: Some(scroll_y.unsigned_abs() as i32),
            direction: Some(direction.to_string()),
        };
        let resp = computer_use_api::scroll(self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }

    /// Drag from one position to another.
    pub async fn drag(
        &self,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Result<daytona_toolbox_client::models::MouseDragResponse, DaytonaError> {
        let req = daytona_toolbox_client::models::MouseDragRequest {
            start_x: Some(start_x),
            start_y: Some(start_y),
            end_x: Some(end_x),
            end_y: Some(end_y),
            button: None,
        };
        let resp = computer_use_api::drag(self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }
}

/// Keyboard operations.
pub struct KeyboardService<'a> {
    config: &'a ToolboxConfig,
}

impl KeyboardService<'_> {
    /// Type text.
    ///
    /// The optional `delay` parameter specifies milliseconds between keystrokes,
    /// matching Go/TypeScript SDK behavior.
    pub async fn type_text(&self, text: &str, delay: Option<i32>) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::KeyboardTypeRequest {
            text: Some(text.to_string()),
            delay,
        };
        computer_use_api::type_text(self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Press a single key with optional modifier keys.
    ///
    /// The optional `modifiers` parameter specifies modifier keys to hold
    /// (e.g., `["ctrl", "shift"]`), matching Go/TypeScript SDK behavior.
    pub async fn press(
        &self,
        key: &str,
        modifiers: Option<Vec<String>>,
    ) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::KeyboardPressRequest {
            key: Some(key.to_string()),
            modifiers,
        };
        computer_use_api::press_key(self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Press a hotkey combination (e.g., "ctrl+c", "cmd+v").
    pub async fn hotkey(&self, keys: &str) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::KeyboardHotkeyRequest {
            keys: Some(keys.to_string()),
        };
        computer_use_api::press_hotkey(self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }
}

/// Screenshot operations.
pub struct ScreenshotService<'a> {
    config: &'a ToolboxConfig,
}

impl ScreenshotService<'_> {
    /// Take a full screen screenshot.
    pub async fn take_full_screen(
        &self,
    ) -> Result<daytona_toolbox_client::models::ScreenshotResponse, DaytonaError> {
        let resp = computer_use_api::take_screenshot(self.config, None)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }

    /// Take a screenshot of a specific region.
    pub async fn take_region(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<daytona_toolbox_client::models::ScreenshotResponse, DaytonaError> {
        let resp = computer_use_api::take_region_screenshot(self.config, x, y, width, height, None)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }
}

/// Display operations.
pub struct DisplayService<'a> {
    config: &'a ToolboxConfig,
}

impl DisplayService<'_> {
    /// Get display information.
    pub async fn get_info(
        &self,
    ) -> Result<daytona_toolbox_client::models::DisplayInfoResponse, DaytonaError> {
        let info = computer_use_api::get_display_info(self.config)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(info)
    }

    /// Get window list.
    pub async fn get_windows(
        &self,
    ) -> Result<daytona_toolbox_client::models::WindowsResponse, DaytonaError> {
        let windows = computer_use_api::get_windows(self.config)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(windows)
    }
}

/// Recording operations.
pub struct RecordingService<'a> {
    config: &'a ToolboxConfig,
}

impl RecordingService<'_> {
    /// Start a recording.
    ///
    /// The optional `label` parameter sets a descriptive label for the recording,
    /// matching Go/TypeScript SDK behavior.
    pub async fn start(
        &self,
        label: Option<&str>,
    ) -> Result<daytona_toolbox_client::models::Recording, DaytonaError> {
        let req = label.map(|l| daytona_toolbox_client::models::StartRecordingRequest {
            label: Some(l.to_string()),
        });
        let resp = computer_use_api::start_recording(self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }

    /// Stop the current recording.
    pub async fn stop(
        &self,
        recording_id: &str,
    ) -> Result<daytona_toolbox_client::models::Recording, DaytonaError> {
        let req = daytona_toolbox_client::models::StopRecordingRequest {
            id: recording_id.to_string(),
        };
        let resp = computer_use_api::stop_recording(self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }

    /// List all recordings.
    pub async fn list(
        &self,
    ) -> Result<daytona_toolbox_client::models::ListRecordingsResponse, DaytonaError> {
        let recordings = computer_use_api::list_recordings(self.config)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(recordings)
    }

    /// Get a specific recording.
    pub async fn get(
        &self,
        recording_id: &str,
    ) -> Result<daytona_toolbox_client::models::Recording, DaytonaError> {
        let recording = computer_use_api::get_recording(self.config, recording_id)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(recording)
    }

    /// Delete a recording.
    pub async fn delete(&self, recording_id: &str) -> Result<(), DaytonaError> {
        computer_use_api::delete_recording(self.config, recording_id)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Download a recording file and save it to a local path.
    ///
    /// The file is streamed directly to disk. Matches Go SDK's
    /// `Recording.Download(ctx, id, localPath)` and TypeScript SDK's
    /// `recording.download(id, localPath)`.
    pub async fn download(
        &self,
        recording_id: &str,
        local_path: &std::path::Path,
    ) -> Result<(), DaytonaError> {
        let resp = computer_use_api::download_recording(self.config, recording_id)
            .await
            .map_err(convert_toolbox_error)?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| DaytonaError::general(format!("failed to read recording data: {}", e)))?;

        // Create parent directory if needed
        if let Some(parent) = local_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    DaytonaError::general(format!("failed to create parent directory: {}", e))
                })?;
            }
        }

        std::fs::write(local_path, &bytes).map_err(|e| {
            DaytonaError::general(format!("failed to write recording file: {}", e))
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn cu_service(mock_server: &MockServer) -> ComputerUseService {
        let config = ToolboxConfig {
            base_path: mock_server.uri(),
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            user_agent: None,
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: Some("test-token".to_string()),
            api_key: None,
        };
        ComputerUseService { config }
    }

    #[tokio::test]
    async fn test_start_stop_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/computeruse/start"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"message": "started"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/computeruse/stop"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"message": "stopped"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/computeruse/process-status"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "running"})),
            )
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        svc.start().await.unwrap();
        svc.get_status().await.unwrap();
        svc.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_mouse_operations() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/computeruse/mouse/position"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"x": 100, "y": 200})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/computeruse/mouse/move"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"x": 300, "y": 400})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/computeruse/mouse/click"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"x": 300, "y": 400})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/computeruse/mouse/scroll"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"success": true})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/computeruse/mouse/drag"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"x": 500, "y": 500})),
            )
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        let mouse = svc.mouse();

        mouse.get_position().await.unwrap();
        mouse.r#move(300, 400).await.unwrap();
        mouse.click(300, 400, "left", None).await.unwrap();
        mouse.scroll(300, 400, 0, -3).await.unwrap();
        mouse.drag(100, 100, 500, 500).await.unwrap();
    }

    #[tokio::test]
    async fn test_keyboard_operations() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/computeruse/keyboard/type"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/computeruse/keyboard/key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/computeruse/keyboard/hotkey"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        let kb = svc.keyboard();

        kb.type_text("Hello, world!", None).await.unwrap();
        kb.press("Enter", None).await.unwrap();
        kb.hotkey("ctrl+c").await.unwrap();
    }

    #[tokio::test]
    async fn test_screenshot_operations() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/computeruse/screenshot"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "screenshot": "iVBOR..."
            })))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/computeruse/screenshot/region"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "screenshot": "iVBOR..."
            })))
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        let ss = svc.screenshot();

        ss.take_full_screen().await.unwrap();
        ss.take_region(0, 0, 800, 600).await.unwrap();
    }

    #[tokio::test]
    async fn test_display_operations() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/computeruse/display/info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "displays": [{"width": 1920, "height": 1080}]
            })))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/computeruse/display/windows"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "windows": [
                    {"title": "Terminal", "id": 1234}
                ]
            })))
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        let display = svc.display();

        display.get_info().await.unwrap();
        display.get_windows().await.unwrap();
    }

    #[tokio::test]
    async fn test_recording_operations() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/computeruse/recordings/start"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": "rec-1", "status": "recording", "fileName": "rec-1.mp4", "filePath": "/tmp/rec-1.mp4", "startTime": "2024-01-01T00:00:00Z"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/computeruse/recordings/stop"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": "rec-1", "status": "stopped", "fileName": "rec-1.mp4", "filePath": "/tmp/rec-1.mp4", "startTime": "2024-01-01T00:00:00Z"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/computeruse/recordings"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"recordings": [{"id": "rec-1", "status": "stopped", "fileName": "rec-1.mp4", "filePath": "/tmp/rec-1.mp4", "startTime": "2024-01-01T00:00:00Z"}]})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/computeruse/recordings/rec-1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": "rec-1", "status": "stopped", "fileName": "rec-1.mp4", "filePath": "/tmp/rec-1.mp4", "startTime": "2024-01-01T00:00:00Z"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("DELETE"))
            .and(path("/computeruse/recordings/rec-1"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        let rec = svc.recording();

        rec.start(None).await.unwrap();
        rec.stop("rec-1").await.unwrap();
        rec.list().await.unwrap();
        rec.get("rec-1").await.unwrap();
        rec.delete("rec-1").await.unwrap();
    }

    #[tokio::test]
    async fn test_mouse_double_click() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/computeruse/mouse/click"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"x": 300, "y": 400})),
            )
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        let mouse = svc.mouse();
        mouse.click(300, 400, "left", Some(true)).await.unwrap();
    }

    #[tokio::test]
    async fn test_keyboard_type_with_delay() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/computeruse/keyboard/type"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        let kb = svc.keyboard();
        kb.type_text("slow typing", Some(50)).await.unwrap();
    }

    #[tokio::test]
    async fn test_keyboard_press_with_modifiers() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/computeruse/keyboard/key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        let kb = svc.keyboard();
        kb.press("c", Some(vec!["ctrl".to_string()]))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_recording_start_with_label() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/computeruse/recordings/start"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": "rec-2", "status": "recording", "fileName": "rec-2.mp4", "filePath": "/tmp/rec-2.mp4", "startTime": "2024-01-01T00:00:00Z"})),
            )
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        let rec = svc.recording();
        let recording = rec.start(Some("test-recording")).await.unwrap();
        assert_eq!(recording.id, "rec-2");
    }

    #[tokio::test]
    async fn test_recording_download() {
        let mock_server = MockServer::start().await;

        let recording_data = b"fake-video-data";
        Mock::given(method("GET"))
            .and(path("/computeruse/recordings/rec-1/download"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(recording_data.to_vec()),
            )
            .mount(&mock_server)
            .await;

        let svc = cu_service(&mock_server).await;
        let rec = svc.recording();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let local_path = tmp.path().to_path_buf();
        rec.download("rec-1", &local_path).await.unwrap();
        let saved = std::fs::read(&local_path).unwrap();
        assert_eq!(saved, recording_data);
    }
}