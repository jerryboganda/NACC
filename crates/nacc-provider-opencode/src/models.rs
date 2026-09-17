//! Gateway model discovery (master plan S9.5's "model endpoint discovery").
//! The wire format is the OpenAI-compatible `GET {base_url}/models`
//! document; the HTTP fetch itself sits behind a one-method trait so the
//! parsing and the "show exactly what the gateway returned, invent nothing"
//! rule are testable without a network.

use serde::Deserialize;

/// What a model-discovery response looks like once parsed. `id` is verbatim
/// from the gateway -- never normalized, never hard-coded (S9.5: "Do not
/// hard-code ... or assume an exact model spelling").
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredModel {
    pub id: String,
    pub display_alias: Option<String>,
}

/// The fetch seam. A real implementation performs `GET {base}/models` with
/// the profile's credential header; tests substitute canned documents.
pub trait FetchModels: Send + Sync {
    fn fetch(&self, base_url: &str) -> Result<String, String>;
}

#[derive(Debug, Deserialize)]
struct ModelsDocument {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    #[allow(dead_code)]
    display_name: Option<String>,
}

/// Parse a discovery document. Unknown fields are ignored on purpose: a
/// gateway that returns extra metadata must not break discovery.
pub fn parse_models_json(document: &str) -> Result<Vec<DiscoveredModel>, String> {
    let parsed: ModelsDocument =
        serde_json::from_str(document).map_err(|e| format!("unrecognized models document: {e}"))?;
    Ok(parsed
        .data
        .into_iter()
        .map(|entry| DiscoveredModel {
            id: entry.id,
            display_alias: entry.display_name,
        })
        .collect())
}

/// Discover models for a profile: the gateway's own list plus the user's
/// manually added IDs (deduplicated, gateway order first). When the gateway
/// is unreachable, the manual IDs alone are the answer -- discovery being
/// unavailable must not hide the models the user typed.
pub fn discover(
    fetcher: &dyn FetchModels,
    base_url: &str,
    manual_model_ids: &[String],
) -> Result<Vec<DiscoveredModel>, String> {
    let mut models = match fetcher.fetch(base_url) {
        Ok(document) => parse_models_json(&document)?,
        Err(err) => {
            if manual_model_ids.is_empty() {
                return Err(format!(
                    "model discovery failed and no manual model ids are configured: {err}"
                ));
            }
            Vec::new()
        }
    };
    for manual in manual_model_ids {
        if !models.iter().any(|model| &model.id == manual) {
            models.push(DiscoveredModel {
                id: manual.clone(),
                display_alias: None,
            });
        }
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Canned(&'static str);

    impl FetchModels for Canned {
        fn fetch(&self, _base_url: &str) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    struct Broken;

    impl FetchModels for Broken {
        fn fetch(&self, _base_url: &str) -> Result<String, String> {
            Err("connection refused".to_string())
        }
    }

    const DOCUMENT: &str = r#"{"object":"list","data":[
        {"id":"glm-family-large","display_name":"GLM Large","owned_by":"x"},
        {"id":"deepseek-v3"}
    ]}"#;

    #[test]
    fn models_are_reported_exactly_as_the_gateway_returned_them() {
        let models = parse_models_json(DOCUMENT).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "glm-family-large");
        assert_eq!(models[0].display_alias.as_deref(), Some("GLM Large"));
        assert_eq!(models[1].display_alias, None, "no alias is invented");
    }

    #[test]
    fn a_foreign_document_is_a_typed_error_not_a_guess() {
        assert!(parse_models_json("<html>gateway error page</html>").is_err());
    }

    #[test]
    fn manual_ids_survive_a_dead_gateway_and_duplicates_are_dropped() {
        let manual = vec!["glm-family-large".to_string(), "my-own-model".to_string()];
        let models = discover(&Broken, "https://gw.example/v1", &manual).unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["glm-family-large", "my-own-model"]
        );
        let models = discover(&Canned(DOCUMENT), "https://gw.example/v1", &manual).unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["glm-family-large", "deepseek-v3", "my-own-model"]
        );
    }

    #[test]
    fn a_dead_gateway_with_no_manual_ids_is_an_error_not_an_empty_list() {
        let err = discover(&Broken, "https://gw.example/v1", &[]).unwrap_err();
        assert!(err.contains("discovery failed"));
    }
}
