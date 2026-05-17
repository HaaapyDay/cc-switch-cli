use crate::provider::Provider;
use serde_json::Value;

pub struct ModelMapping {
    pub haiku_model: Option<String>,
    pub sonnet_model: Option<String>,
    pub opus_model: Option<String>,
    pub default_model: Option<String>,
    pub reasoning_model: Option<String>,
}

impl ModelMapping {
    pub fn from_provider(provider: &Provider) -> Self {
        let env = provider.settings_config.get("env");

        Self {
            haiku_model: env
                .and_then(|value| value.get("ANTHROPIC_DEFAULT_HAIKU_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            sonnet_model: env
                .and_then(|value| value.get("ANTHROPIC_DEFAULT_SONNET_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            opus_model: env
                .and_then(|value| value.get("ANTHROPIC_DEFAULT_OPUS_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            default_model: env
                .and_then(|value| value.get("ANTHROPIC_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            reasoning_model: env
                .and_then(|value| value.get("ANTHROPIC_REASONING_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
        }
    }

    pub fn has_mapping(&self) -> bool {
        self.haiku_model.is_some()
            || self.sonnet_model.is_some()
            || self.opus_model.is_some()
            || self.default_model.is_some()
            || self.reasoning_model.is_some()
    }

    fn provider_model_values(&self) -> impl Iterator<Item = &str> {
        [
            self.haiku_model.as_deref(),
            self.sonnet_model.as_deref(),
            self.opus_model.as_deref(),
            self.default_model.as_deref(),
            self.reasoning_model.as_deref(),
        ]
        .into_iter()
        .flatten()
    }

    fn is_provider_model_value(&self, original_model: &str) -> bool {
        self.provider_model_values()
            .any(|model| model.trim() == original_model.trim())
    }

    pub fn map_model(&self, original_model: &str, has_thinking: bool) -> String {
        let model_lower = original_model.to_lowercase();

        if self.is_provider_model_value(original_model) {
            return original_model.to_string();
        }

        if has_thinking {
            if let Some(model) = &self.reasoning_model {
                return model.clone();
            }
        }

        if model_lower.contains("haiku") {
            if let Some(model) = &self.haiku_model {
                return model.clone();
            }
        }
        if model_lower.contains("opus") {
            if let Some(model) = &self.opus_model {
                return model.clone();
            }
        }
        if model_lower.contains("sonnet") {
            if let Some(model) = &self.sonnet_model {
                return model.clone();
            }
        }

        if let Some(model) = &self.default_model {
            return model.clone();
        }

        original_model.to_string()
    }
}

pub fn has_thinking_enabled(body: &Value) -> bool {
    match body
        .get("thinking")
        .and_then(Value::as_object)
        .and_then(|object| object.get("type"))
        .and_then(Value::as_str)
    {
        Some("enabled") | Some("adaptive") => true,
        Some("disabled") | None => false,
        Some(other) => {
            log::warn!(
                "[ModelMapper] unknown thinking.type='{other}', treat as disabled to avoid misrouting reasoning model"
            );
            false
        }
    }
}

pub fn apply_model_mapping(
    mut body: Value,
    provider: &Provider,
) -> (Value, Option<String>, Option<String>) {
    let mapping = ModelMapping::from_provider(provider);

    if !mapping.has_mapping() {
        let original = body.get("model").and_then(Value::as_str).map(String::from);
        return (body, original, None);
    }

    let original_model = body.get("model").and_then(Value::as_str).map(String::from);

    if let Some(original) = &original_model {
        let mapped = mapping.map_model(original, has_thinking_enabled(&body));

        if mapped != *original {
            body["model"] = serde_json::json!(mapped);
            return (body, Some(original.clone()), Some(mapped));
        }
    }

    (body, original_model, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider_with_mapping() -> Provider {
        Provider::with_id(
            "mapped".to_string(),
            "Mapped".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_MODEL": "gpt-5.4",
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "gpt-5.4-mini",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL": "gpt-5.4",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL": "gpt-5.4"
                }
            }),
            None,
        )
    }

    #[test]
    fn actual_provider_model_passes_through_without_default_remap() {
        let provider = provider_with_mapping();
        let body = json!({"model": "gpt-5.4-mini"});

        let (mapped_body, original, mapped) = apply_model_mapping(body, &provider);

        assert_eq!(mapped_body["model"], "gpt-5.4-mini");
        assert_eq!(original.as_deref(), Some("gpt-5.4-mini"));
        assert_eq!(mapped, None);
    }

    #[test]
    fn legacy_claude_role_alias_still_maps_to_provider_model() {
        let provider = provider_with_mapping();
        let body = json!({"model": "claude-haiku-4-5"});

        let (mapped_body, original, mapped) = apply_model_mapping(body, &provider);

        assert_eq!(mapped_body["model"], "gpt-5.4-mini");
        assert_eq!(original.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(mapped.as_deref(), Some("gpt-5.4-mini"));
    }
}
