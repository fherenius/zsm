mod new_session_info;
mod session;
mod state;
mod ui;
mod zoxide;

use state::PluginState;
use std::collections::BTreeMap;
use ui::PluginRenderer;
use zellij_tile::prelude::*;
use zsm::naming;

register_plugin!(PluginState);

impl ZellijPlugin for PluginState {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.initialize(configuration);

        // Request permissions - same as session-manager
        request_permission(&[
            PermissionType::RunCommands,                  // run zoxide command
            PermissionType::ReadApplicationState,         // read current sessions/layouts
            PermissionType::ChangeApplicationState,       // create and switch sessions
            PermissionType::MessageAndLaunchOtherPlugins, // launch filepicker plugin
        ]);

        subscribe(&[
            EventType::ModeUpdate,
            EventType::SessionUpdate,
            EventType::Key,
            EventType::RunCommandResult,
            EventType::PermissionRequestResult,
            // Re-fetch zoxide directories whenever the plugin is reopened/focused
            EventType::Visible,
        ]);

        // Don't fetch zoxide directories immediately - wait for permissions
    }

    // Event handlers
    fn update(&mut self, event: Event) -> bool {
        let mut should_render = false;

        match event {
            Event::ModeUpdate(mode_info) => {
                self.set_colors(mode_info.style.colors.into());
                should_render = true;
            }
            Event::Key(key) => {
                should_render = self.handle_key(key);
            }
            Event::PermissionRequestResult(permission_status) => {
                match permission_status {
                    PermissionStatus::Granted => {
                        // Now that we have permissions, fetch zoxide directories
                        self.fetch_zoxide_directories();
                        // Pull the full session list. The passive SessionUpdate event
                        // only ever carries the current session until a plugin actively
                        // requests the list (Zellij 0.44 API model), so we must pull it.
                        self.fetch_sessions();
                        should_render = true;
                    }
                    PermissionStatus::Denied => {
                        self.set_error(
                            "RunCommands permission denied - cannot fetch zoxide directories"
                                .to_string(),
                        );
                        should_render = true;
                    }
                }
            }
            Event::SessionUpdate(session_infos, resurrectable_session_infos) => {
                self.update_sessions(session_infos);
                self.update_resurrectable_sessions(resurrectable_session_infos);
                should_render = true;
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context)
                if context.contains_key("zoxide_query") =>
            {
                if exit_code == Some(0) {
                    let stdout_str = String::from_utf8_lossy(&stdout);
                    self.process_zoxide_output(&stdout_str);
                    should_render = true;
                } else {
                    let stderr_str = String::from_utf8_lossy(&stderr);
                    self.set_error(format!(
                        "Failed to run zoxide (is it installed?): {}",
                        stderr_str
                    ));
                    should_render = true;
                }
            }
            Event::Visible(true) => {
                // Plugin was (re)opened or focused - refresh the zoxide list so it
                // reflects directories visited since it was last shown, and re-pull
                // the session list (it may have changed while we were hidden).
                self.fetch_zoxide_directories();
                self.fetch_sessions();
                should_render = true;
            }
            _ => (),
        }

        should_render
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        // Handle filepicker results for new session creation
        if pipe_message.name != "filepicker_result" {
            return false;
        }

        let mut should_render = false;
        if let (Some(payload), Some(request_id)) =
            (pipe_message.payload, pipe_message.args.get("request_id"))
        {
            // Check if this request ID is valid for our plugin
            if self.is_valid_request_id(request_id) {
                self.remove_request_id(request_id);

                // Use the picked path as-is. Probing it with `exists()` and
                // `is_file()` cannot work from here: the plugin only sees its
                // WASI preopens (/host, /data, /tmp), never an arbitrary host
                // path, so those checks always reported "missing" and the
                // extension fallback ran instead - which took the *parent* of
                // any directory with a dot in its name, turning
                // ~/projects/site.com into ~/projects. The filepicker is
                // launched asking for a folder, so the path is the folder.
                self.set_new_session_folder(Some(std::path::PathBuf::from(payload)));
                should_render = true;
            }
        }

        should_render
    }

    fn render(&mut self, rows: usize, cols: usize) {
        PluginRenderer::render(self, rows, cols);
    }
}

impl PluginState {
    fn fetch_zoxide_directories(&mut self) {
        let mut context = BTreeMap::new();
        context.insert("zoxide_query".to_string(), "true".to_string());
        run_command(&["zoxide", "query", "-l", "-s"], context);
    }

    /// Pull the full session list directly from Zellij via `get_session_list()`.
    ///
    /// Zellij only refreshes the server-side peer-session cache (the source of the
    /// `SessionUpdate` event) when a plugin actively calls `get_session_list()`.
    /// Subscribing to `SessionUpdate` alone yields only the current session, so we
    /// pull the list explicitly (this also primes the cache, so subsequent
    /// `SessionUpdate` events become complete). Mirrors the built-in session-manager.
    fn fetch_sessions(&mut self) {
        match get_session_list() {
            Ok(snapshot) => {
                self.update_sessions(snapshot.live_sessions);
                self.update_resurrectable_sessions(snapshot.resurrectable_sessions);
            }
            Err(e) => {
                eprintln!("[zsm] get_session_list failed: {}", e);
            }
        }
    }

    fn process_zoxide_output(&mut self, output: &str) {
        let mut directories = Vec::new();

        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }

            // zoxide output format: "score path"
            let parts: Vec<&str> = line.trim().splitn(2, ' ').collect();
            if parts.len() == 2 {
                if let Ok(score) = parts[0].parse::<f64>() {
                    let path = parts[1];

                    directories.push(zoxide::ZoxideDirectory {
                        ranking: score,
                        directory: path.to_string(),
                        session_name: String::new(), // Will be set by smart naming
                    });
                }
            }
        }

        // Generate smart session names before sorting
        let names = {
            let paths: Vec<&str> = directories
                .iter()
                .map(|directory| directory.directory.as_str())
                .collect();
            naming::session_names(&paths, self.config())
        };
        for (directory, name) in directories.iter_mut().zip(names) {
            directory.session_name = name;
        }

        // Sort by score in descending order (higher scores first)
        directories.sort_by(|a, b| {
            b.ranking
                .partial_cmp(&a.ranking)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        self.update_zoxide_directories(directories);
    }
}
