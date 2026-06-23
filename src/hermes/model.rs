use serde::Deserialize;

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
}
