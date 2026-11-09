use crate::models::ConfirmationChallenge;
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

struct Pending {
    action: String,
    description: String,
    expires_at: Instant,
}

struct Approved {
    action: String,
    expires_at: Instant,
}

#[derive(Default)]
pub struct ConfirmationManager {
    pending: Mutex<HashMap<String, Pending>>,
    approved: Mutex<HashMap<String, Approved>>,
}

impl ConfirmationManager {
    pub fn request(&self, action: String, description: String) -> Result<ConfirmationChallenge> {
        if action.len() > 1_000 || description.len() > 2_000 {
            bail!("Confirmarea este prea lungă");
        }
        let id = Uuid::new_v4().to_string();
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("Confirmările sunt blocate"))?
            .insert(
                id.clone(),
                Pending {
                    action,
                    description: description.clone(),
                    expires_at: Instant::now() + Duration::from_secs(120),
                },
            );
        Ok(ConfirmationChallenge { id, description })
    }

    pub fn cancel(&self, id: &str) -> Result<()> {
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("Confirmările sunt blocate"))?
            .remove(id);
        Ok(())
    }

    pub fn approve(&self, id: &str) -> Result<String> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("Confirmările sunt blocate"))?
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("Confirmarea nu există sau a fost deja folosită"))?;
        if Instant::now() > pending.expires_at {
            bail!("Confirmarea a expirat");
        }
        let _ = pending.description;
        let token = Uuid::new_v4().to_string();
        self.approved
            .lock()
            .map_err(|_| anyhow::anyhow!("Confirmările sunt blocate"))?
            .insert(
                token.clone(),
                Approved {
                    action: pending.action,
                    expires_at: Instant::now() + Duration::from_secs(60),
                },
            );
        Ok(token)
    }

    pub fn consume(&self, token: &str, expected_action: &str) -> Result<()> {
        let approved = self
            .approved
            .lock()
            .map_err(|_| anyhow::anyhow!("Confirmările sunt blocate"))?
            .remove(token)
            .ok_or_else(|| {
                anyhow::anyhow!("Tokenul de confirmare lipsește, a expirat sau a fost folosit")
            })?;
        if Instant::now() > approved.expires_at {
            bail!("Tokenul de confirmare a expirat");
        }
        if approved.action != expected_action {
            bail!("Confirmarea nu corespunde acțiunii cerute");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_scoped_and_consumed_once() {
        let manager = ConfirmationManager::default();
        let challenge = manager.request("write:a".into(), "test".into()).unwrap();
        let token = manager.approve(&challenge.id).unwrap();
        assert!(manager.consume(&token, "write:b").is_err());
        assert!(manager.consume(&token, "write:a").is_err());
    }
}
