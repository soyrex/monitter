//! Owner-local voice lease. A service instance is the profile boundary.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceLease {
    pub task_id: String,
    pub client_id: String,
    pub epoch: String,
}

#[derive(Default)]
pub(crate) struct VoiceSessions {
    pub current: Option<VoiceLease>,
    expires_at: Option<std::time::Instant>,
}

impl VoiceSessions {
    pub fn begin(&mut self, task_id: String, client_id: String) -> Result<VoiceLease, String> {
        let parsed = uuid::Uuid::parse_str(&client_id).map_err(|_| "Invalid voice client ID.")?;
        if parsed.to_string() != client_id {
            return Err("Voice client ID must use canonical UUID form.".into());
        }
        let lease = VoiceLease {
            task_id,
            client_id,
            epoch: crate::model::id(),
        };
        self.current = Some(lease.clone());
        self.expires_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(15));
        Ok(lease)
    }
    pub fn check(&self, lease: &VoiceLease) -> Result<(), String> {
        if self.current.as_ref() == Some(lease)
            && self
                .expires_at
                .is_some_and(|expiry| std::time::Instant::now() < expiry)
        {
            Ok(())
        } else {
            Err("Voice session is no longer current.".into())
        }
    }
    pub fn renew(&mut self, lease: &VoiceLease) -> Result<(), String> {
        self.check(lease)?;
        self.expires_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(15));
        Ok(())
    }
    pub fn end(&mut self, lease: &VoiceLease) -> Result<(), String> {
        self.check(lease)?;
        self.current = None;
        self.expires_at = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_replacement_and_end_reject_stale_epochs() {
        let mut sessions = VoiceSessions::default();
        let client = uuid::Uuid::new_v4().to_string();
        let old = sessions.begin("task-a".into(), client.clone()).unwrap();
        let next = sessions.begin("task-b".into(), client).unwrap();
        assert!(sessions.check(&old).is_err());
        assert!(sessions.end(&old).is_err());
        assert!(sessions.check(&next).is_ok());
        sessions.expires_at = Some(std::time::Instant::now());
        assert!(sessions.renew(&next).is_err());
        let next = sessions.begin("task-b".into(), next.client_id).unwrap();
        sessions.end(&next).unwrap();
        assert!(sessions.check(&next).is_err());
    }
}
