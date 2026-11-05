use super::{HumanRender, OutputMode};
use schemars::JsonSchema;
use serde::Serialize;

/// The named top-level field(s) of a serialized response: plain text for
/// `--field`, one compact JSON object for `--fields`.
#[derive(Serialize, JsonSchema)]
#[serde(transparent)]
pub struct FieldProjection {
    value: serde_json::Value,
    #[serde(skip)]
    #[schemars(skip)]
    text: String,
}

impl FieldProjection {
    /// Project `name` from the serialized response `shown`; `None` when
    /// the response has no such field.
    pub fn field(shown: &serde_json::Value, name: &str) -> Option<Self> {
        Some(Self {
            value: shown.get(name)?.clone(),
            text: super::project_field(shown, name)?,
        })
    }

    /// Project `names`, reporting the first one the response lacks.
    pub fn fields(
        shown: &serde_json::Value,
        names: &[String],
    ) -> Result<Self, super::UnknownFieldError> {
        Ok(Self {
            text: super::project_fields(shown, names)?,
            value: serde_json::Value::Object(
                names
                    .iter()
                    .filter_map(|name| Some((name.clone(), shown.get(name)?.clone())))
                    .collect(),
            ),
        })
    }
}

impl HumanRender for FieldProjection {
    fn render_human(&self, _: OutputMode) -> String {
        format!("{}\n", self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_projection_serializes_as_the_projected_value() {
        let shown = serde_json::json!({"state": "ready", "title": "T"});

        let field = FieldProjection::field(&shown, "state").unwrap();
        let fields = FieldProjection::fields(&shown, &["title".to_owned()]).unwrap();

        assert_eq!(serde_json::to_value(&field).unwrap(), "ready");
        assert_eq!(
            serde_json::to_value(&fields).unwrap(),
            serde_json::json!({"title": "T"})
        );
        assert_eq!(field.render_human(OutputMode::default()), "ready\n");
    }
}
