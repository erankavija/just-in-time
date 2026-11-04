//! `jit version` handler, which runs before repository bootstrap.

use super::Outcome;
use crate::build_info::{version_info, VersionInfo};

fn show() -> anyhow::Result<Outcome<VersionInfo>> {
    Ok(Outcome::success(version_info()))
}

command_group! {
    command: (),
    leaves: {
        "version" => () => show() -> VersionInfo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{ExitCode, HumanRender, OutputMode};

    fn run(json: bool) -> (ExitCode, String) {
        let mut out = Vec::new();
        let status = dispatch(
            (),
            OutputMode { json, quiet: false },
            &mut out,
            &mut Vec::new(),
        )
        .unwrap();
        (status, String::from_utf8(out).unwrap())
    }

    #[test]
    fn test_dispatch_json_writes_version_info_as_success_envelope() {
        let (status, out) = run(true);

        let printed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(printed, serde_json::to_value(version_info()).unwrap());
        assert_eq!(status, ExitCode::Success);
    }

    #[test]
    fn test_dispatch_human_writes_version_info_rendering() {
        let (status, out) = run(false);

        assert_eq!(
            out,
            version_info().render_human(OutputMode {
                json: false,
                quiet: false
            })
        );
        assert_eq!(status, ExitCode::Success);
    }

    #[test]
    fn test_registered_output_schema_version_is_named_for_version_info() {
        let registered = crate::dispatch::registered_output_schema("version").unwrap();

        assert_eq!(registered.success, "VersionInfo");
    }
}
