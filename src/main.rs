mod new_session_info;
mod session;
mod state;
mod ui;
mod zoxide;

use state::PluginState;
use std::collections::BTreeMap;
use ui::PluginRenderer;
use zellij_tile::prelude::*;

register_plugin!(PluginState);

impl ZellijPlugin for PluginState {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.initialize(configuration);
        self.plugin_id = Some(get_plugin_ids().plugin_id);

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
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::Key,
            EventType::RunCommandResult,
            EventType::PermissionRequestResult,
            // Re-fetch zoxide directories whenever the plugin is reopened/focused
            EventType::Visible,
            EventType::Timer,
        ]);

        // Don't fetch zoxide directories immediately - wait for permissions
    }

    // Event handlers
    fn update(&mut self, event: Event) -> bool {
        let mut should_render = false;

        match event {
            // The UI uses indexed colours, which Zellij resolves against the
            // user's theme, so there is no palette to store - a theme change
            // just needs a repaint.
            Event::ModeUpdate(_) => {
                should_render = true;
            }
            Event::Key(key) => {
                should_render = self.handle_key(key);
            }
            Event::PermissionRequestResult(permission_status) => {
                match permission_status {
                    PermissionStatus::Granted => {
                        self.permissions_granted = true;
                        // Now that we have permissions, fetch zoxide directories
                        self.fetch_zoxide_directories();
                        // Pull metadata and socket-confirmed session membership.
                        self.fetch_sessions();
                        self.refresh_session_usage();
                        self.schedule_session_refresh();
                        should_render = true;
                    }
                    PermissionStatus::Denied => {
                        self.permissions_granted = false;
                        self.set_error(
                            "RunCommands permission denied - cannot fetch zoxide directories"
                                .to_string(),
                        );
                        should_render = true;
                    }
                }
            }
            Event::SessionUpdate(session_infos, _) => {
                // Notifications can precede a newer explicit snapshot. Never
                // replace its membership with the host's stale peer cache.
                self.update_session_notification(session_infos);
                should_render = true;
            }
            Event::PaneUpdate(panes) => {
                if self.update_panes(panes) {
                    self.refresh_picker();
                    should_render = true;
                }
            }
            Event::TabUpdate(tabs) => {
                if self.update_tabs(tabs) {
                    self.refresh_picker();
                    should_render = true;
                }
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context)
                if context.contains_key("session_listing") =>
            {
                self.session_listing_pending = false;
                if exit_code == Some(0) {
                    match self.update_session_listing(&String::from_utf8_lossy(&stdout)) {
                        Ok(()) => should_render = true,
                        Err(error) => eprintln!("[zsm] session listing failed: {error}"),
                    }
                } else {
                    eprintln!(
                        "[zsm] session listing failed: {}",
                        String::from_utf8_lossy(&stderr)
                    );
                }
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
            Event::RunCommandResult(exit_code, stdout, stderr, context)
                if context.contains_key("session_usage") =>
            {
                if exit_code == Some(0) {
                    self.update_session_usage(&String::from_utf8_lossy(&stdout));
                    should_render = true;
                } else {
                    eprintln!(
                        "[zsm] session usage cache failed: {}",
                        String::from_utf8_lossy(&stderr)
                    );
                }
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context)
                if context.contains_key("projects") =>
            {
                self.project_command_finished(
                    exit_code == Some(0),
                    &String::from_utf8_lossy(&stdout),
                    &String::from_utf8_lossy(&stderr),
                    context.get("creation_id").map(String::as_str),
                );
                should_render = true;
            }
            Event::Visible(visible) => {
                self.set_visible(visible);
                // Plugin was (re)opened or focused - refresh the zoxide list so it
                // reflects directories visited since it was last shown, and re-pull
                // the session list (it may have changed while we were hidden).
                if visible {
                    self.refresh_picker();
                    should_render = true;
                }
            }
            Event::Timer(_) => {
                self.refresh_timer_pending = false;
                if self.is_visible && self.permissions_granted {
                    self.fetch_sessions();
                    self.schedule_session_refresh();
                    should_render = true;
                }
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
    fn refresh_picker(&mut self) {
        self.fetch_zoxide_directories();
        self.fetch_sessions();
        self.refresh_session_usage();
        self.schedule_session_refresh();
    }

    fn fetch_zoxide_directories(&mut self) {
        if !self.permissions_granted {
            return;
        }
        let mut context = BTreeMap::new();
        context.insert("zoxide_query".to_string(), "true".to_string());
        run_command(&["zoxide", "query", "-l", "-s"], context);
    }

    /// Pull metadata from the API and membership from the CLI's socket probes.
    fn fetch_sessions(&mut self) {
        if !self.permissions_granted {
            return;
        }
        match get_session_list() {
            Ok(snapshot) => {
                self.update_session_list(snapshot.live_sessions, snapshot.resurrectable_sessions);
            }
            Err(e) => {
                eprintln!("[zsm] get_session_list failed: {}", e);
            }
        }
        if !self.session_listing_pending {
            self.session_listing_pending = true;
            run_command(
                &["zellij", "list-sessions", "--no-formatting"],
                BTreeMap::from([("session_listing".into(), "true".into())]),
            );
        }
    }

    fn schedule_session_refresh(&mut self) {
        // Do not fetch in response to SessionUpdate: get_session_list itself
        // emits that event in Zellij 0.45.1, which would create a feedback loop.
        if self.permissions_granted && self.is_visible && !self.refresh_timer_pending {
            self.refresh_timer_pending = true;
            set_timeout(1.0);
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

        // Most-used first; see ZoxideDirectory's Ord impl.
        directories.sort();

        self.update_zoxide_directories(directories);
    }
}
