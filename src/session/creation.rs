use std::path::PathBuf;
use zellij_tile::prelude::{switch_session_with_cwd, switch_session_with_layout, LayoutInfo};
use zsm::session_name;

/// A creation request is assembled without host calls, then validated and
/// executed by PluginState through the same path for every creation shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreationRequest {
    pub name: String,
    pub folder: Option<PathBuf>,
    pub layout: Option<LayoutInfo>,
}

impl CreationRequest {
    pub fn with_default_layout(
        name: String,
        folder: Option<PathBuf>,
        default_layout: Option<&str>,
        layouts: &[LayoutInfo],
    ) -> Result<Self, &'static str> {
        let layout = default_layout
            .map(|name| {
                layouts
                    .iter()
                    .find(|layout| layout.name() == name)
                    .cloned()
                    .ok_or(
                        "Default layout is unavailable; select a layout or update default_layout",
                    )
            })
            .transpose()?;
        Ok(Self {
            name,
            folder,
            layout,
        })
    }

    pub fn validate(
        &self,
        current: Option<&str>,
        is_taken: impl FnOnce(&str) -> bool,
    ) -> Result<(), &'static str> {
        session_name::validate_for_creation(&self.name, current, is_taken)
    }

    pub fn execute(&self) {
        let name = (!self.name.is_empty()).then_some(self.name.as_str());
        match &self.layout {
            Some(layout) => switch_session_with_layout(name, layout.clone(), self.folder.clone()),
            None => switch_session_with_cwd(name, self.folder.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_is_explicit_and_missing_layouts_are_reported() {
        let layouts = [LayoutInfo::BuiltIn("default".into())];
        let request = CreationRequest::with_default_layout(
            "project".into(),
            Some("/work/project".into()),
            Some("default"),
            &layouts,
        )
        .unwrap();
        assert_eq!(request.layout, Some(layouts[0].clone()));
        assert_eq!(request.folder, Some("/work/project".into()));
        assert!(CreationRequest::with_default_layout(
            "project".into(),
            None,
            Some("missing"),
            &layouts
        )
        .is_err());
        assert!(request.validate(Some("current"), |_| true).is_err());
        assert!(request.validate(Some("project"), |_| false).is_err());
        assert!(request.validate(Some("current"), |_| false).is_ok());
    }
}
