//! Human rendering of the `jit version` response.

use super::HumanRender;
use crate::build_info::VersionInfo;

impl HumanRender for VersionInfo {
    fn render_human(&self) -> String {
        format!(
            "Version: {}\nProfile: {}\nTarget: {}\n",
            self.version, self.build_profile, self.target
        )
    }
}
