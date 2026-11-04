//! Durable admission identity, stored with the canonical message or queue row.
use crate::voice_session::VoiceLease;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceUtterance {
    pub lease: VoiceLease,
    pub utterance_id: String,
    pub text: String,
    pub is_final: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceReceipt {
    pub utterance_id: String,
    pub task_id: String,
    pub row_id: String,
    pub disposition: String,
    pub admitted_at: i64,
    pub delivery_status: String,
    pub message_id: Option<String>,
    pub message_created_at: Option<i64>,
    pub duplicate: bool,
}
impl VoiceUtterance {
    pub fn validate(&self) -> Result<(), String> {
        let parsed =
            uuid::Uuid::parse_str(&self.utterance_id).map_err(|_| "Invalid utterance ID.")?;
        if parsed.to_string() != self.utterance_id {
            return Err("Utterance ID must use canonical UUID form.".into());
        }
        if !self.is_final {
            return Err("Only final speech can be admitted.".into());
        }
        if self.text.trim().is_empty() || self.text.len() > 64 * 1024 {
            return Err("Voice text is empty or too long.".into());
        }
        Ok(())
    }
}

impl crate::Service {
    pub(crate) fn begin_voice_session(
        &self,
        task_id: String,
        client_id: String,
    ) -> Result<VoiceLease, String> {
        let mut sessions = self
            .voice_sessions
            .lock()
            .map_err(|_| "Voice session lock failed.")?;
        let (task, host) = self.task_and_host(&task_id)?;
        if task.archived
            || task.provider != "codex"
            || host.kind != "local"
            || task.channel_id.is_some()
        {
            return Err("Voice requires an unarchived local Codex task.".into());
        }
        sessions.begin(task_id, client_id)
    }
    pub(crate) fn end_voice_session(&self, lease: &VoiceLease) -> Result<(), String> {
        self.voice_sessions
            .lock()
            .map_err(|_| "Voice session lock failed.")?
            .end(lease)
    }
    pub(crate) fn renew_voice_session(&self, lease: &VoiceLease) -> Result<(), String> {
        let mut sessions = self
            .voice_sessions
            .lock()
            .map_err(|_| "Voice session lock failed.")?;
        sessions.check(lease)?;
        let (task, host) = self.task_and_host(&lease.task_id)?;
        if task.archived
            || task.provider != "codex"
            || host.kind != "local"
            || task.channel_id.is_some()
        {
            return Err("Voice task is no longer eligible.".into());
        }
        sessions.renew(lease)
    }
    pub(crate) fn read_voice_receipt(
        &self,
        task_id: &str,
        client_id: &str,
        utterance_id: &str,
    ) -> Result<Option<VoiceReceipt>, String> {
        uuid::Uuid::parse_str(utterance_id).map_err(|_| "Invalid utterance ID.")?;
        let Some((event_task, payload, delivery)) = self.store.voice_admission(utterance_id)?
        else {
            return Ok(None);
        };
        let value: serde_json::Value =
            serde_json::from_str(&payload).map_err(|_| "Invalid voice receipt.")?;
        if event_task != task_id || value["clientId"].as_str() != Some(client_id) {
            return Err("Voice receipt belongs to another session.".into());
        }
        use sha2::{Digest, Sha256};
        let (delivery_status, message_created_at) = match delivery {
            Some((status, text, created_at))
                if value["textDigest"].as_str()
                    == Some(&format!("{:x}", Sha256::digest(text.as_bytes()))) =>
            {
                (status, created_at)
            }
            _ => ("interrupted".into(), None),
        };
        let message_id = (delivery_status == "delivered")
            .then(|| value["rowId"].as_str().unwrap_or_default().to_owned());
        Ok(Some(VoiceReceipt {
            utterance_id: utterance_id.into(),
            task_id: event_task,
            row_id: value["rowId"]
                .as_str()
                .ok_or("Invalid voice receipt.")?
                .into(),
            disposition: value["disposition"]
                .as_str()
                .ok_or("Invalid voice receipt.")?
                .into(),
            admitted_at: value["admittedAt"].as_i64().unwrap_or(0),
            delivery_status,
            message_id,
            message_created_at,
            duplicate: true,
        }))
    }
    pub(crate) fn admit_voice_utterance(
        self: &std::sync::Arc<Self>,
        input: VoiceUtterance,
    ) -> Result<VoiceReceipt, String> {
        input.validate()?;
        // Serialize session changes and all voice admissions through the durable commit.
        let sessions = self
            .voice_sessions
            .lock()
            .map_err(|_| "Voice session lock failed.")?;
        sessions.check(&input.lease)?;
        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(input.text.as_bytes()));
        if let Some((event_task, payload, _)) = self.store.voice_admission(&input.utterance_id)? {
            let value: serde_json::Value =
                serde_json::from_str(&payload).map_err(|_| "Invalid voice receipt.")?;
            if event_task != input.lease.task_id
                || value["clientId"].as_str() != Some(&input.lease.client_id)
                || value["textDigest"].as_str() != Some(&digest)
            {
                return Err("Utterance ID conflicts with an admitted message.".into());
            }
        }
        if let Some(receipt) = self.read_voice_receipt(
            &input.lease.task_id,
            &input.lease.client_id,
            &input.utterance_id,
        )? {
            if receipt.task_id != input.lease.task_id {
                return Err("Utterance belongs to another task.".into());
            }
            return Ok(receipt);
        }
        let (task, host) = self.task_and_host(&input.lease.task_id)?;
        if task.archived
            || task.provider != "codex"
            || host.kind != "local"
            || task.channel_id.is_some()
        {
            return Err("Voice task is no longer eligible.".into());
        }
        let row_id = crate::model::id();
        let accepted = self.accept_send_inner(
            input.lease.task_id.clone(),
            input.text,
            vec![],
            None,
            Some("steer"),
            Some((
                &input.utterance_id,
                &row_id,
                &input.lease.client_id,
                &digest,
            )),
            None,
        )?;
        let mut receipt = self
            .read_voice_receipt(
                &input.lease.task_id,
                &input.lease.client_id,
                &input.utterance_id,
            )?
            .ok_or("Voice admission outcome is uncertain; check its receipt before retrying.")?;
        receipt.duplicate = false;
        drop(sessions);
        self.launch_accepted(input.lease.task_id, accepted);
        Ok(receipt)
    }
}
