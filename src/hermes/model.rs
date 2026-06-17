use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct StatusResponse {
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileList {
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActiveProfile {
    /// The web UI's active-profile endpoint returns `{ "active": "...",
    /// "current": "..." }`. The legacy value is also accepted as a
    /// fallback.
    #[serde(alias = "active")]
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateProfileRequest<'a> {
    pub name: &'a str,
    pub clone_from_default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetSoulRequest<'a> {
    pub content: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_list_deserializes() {
        let json = r#"{"profiles":[{"name":"alpha"},{"name":"beta"}]}"#;
        let parsed: ProfileList = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.profiles.len(), 2);
        assert_eq!(parsed.profiles[0].name, "alpha");
        assert_eq!(parsed.profiles[1].name, "beta");
    }

    #[test]
    fn profile_deserializes() {
        let json = r#"{"name":"trader"}"#;
        let parsed: Profile = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.name, "trader");
    }

    #[test]
    fn active_profile_deserializes_from_name() {
        let json = r#"{"name":"main"}"#;
        let parsed: ActiveProfile = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.name, "main");
    }

    #[test]
    fn active_profile_deserializes_from_active_field() {
        let json = r#"{"active":"main","current":"main"}"#;
        let parsed: ActiveProfile = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.name, "main");
    }

    #[test]
    fn status_response_deserializes() {
        let json = r#"{"version":"0.8.0"}"#;
        let parsed: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.version, "0.8.0");
    }

    #[test]
    fn create_profile_request_serializes() {
        let req = CreateProfileRequest {
            name: "new-agent",
            clone_from_default: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"name\":\"new-agent\""));
        assert!(json.contains("\"clone_from_default\":true"));
    }

    #[test]
    fn set_soul_request_serializes() {
        let req = SetSoulRequest {
            content: "beep boop",
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"content\":\"beep boop\""));
    }
}
