//! The row a log's head shows as a session in lists, the HUD and the
//! session page.
use super::LogHead;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

impl LogHead {
    pub fn session(&self, now: i64, subagents_at: Option<i64>) -> Value {
        let updated = self
            .activity_at(subagents_at)
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .map(|at| at.to_rfc3339())
            .unwrap_or_else(|| self.updated.clone());
        let mut session = json!({"id":self.id,"title":self.title,"source":self.source,"model":self.model,"vendorName":crate::naming::model_names::vendor_name(&self.source),"modelName":(!self.model.is_empty()).then(||crate::naming::model_names::consumer_name(&format!("{}-model:{}",self.source,self.model))),"cwd":self.cwd,"updatedAt":updated,"state":self.live.session_state(now,&self.source,subagents_at),"tokens":self.usage.total(),"cost":self.list_cost_usd(),"contextPercent":null,"logPath":self.origin_path.as_ref().unwrap_or(&self.path),"parentId":self.live.parent_id});
        session["project"] = json!(crate::projects::project_id(self.cwd.as_deref()));
        session["lastMessageAt"] = json!(self
            .last_message_at()
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .map(|at| at.to_rfc3339())
            .unwrap_or_else(|| self.updated.clone()));
        // `tokens` by kind, from the same log: the Sessions page's Today card
        // splits its total by them (agent-hud-native-open `SessionsTodayCard`).
        // These are the log's own; `AgentHudCore::session_totals` adds its
        // sub-agents' wherever a session is shown.
        session["usage"] = self.usage.json();
        if let Some(client) =
            crate::naming::clients::client_name(&self.source, self.client.as_deref())
        {
            session["client"] = json!(client);
        }
        // A far machine's sessions name it; this machine's name none.
        if let Some(machine) = &self.machine {
            session["machine"] = json!(machine);
        }
        // Where its first and latest prompts were typed, as the log tells;
        // the presentation adds what hooks and deliveries report.
        session["origins"] = crate::naming::surfaces::resolve(
            &self.source,
            &self.prompts,
            self.client.as_deref(),
            &Default::default(),
        )
        .json();
        // Its way back, as its agent says; without one an app link may stand
        // in (the presentation adds it).
        if let Some(target) = agent_hud_agents::session_target(
            &self.source,
            &self.id,
            self.cwd.as_deref(),
            self.client.as_deref(),
        ) {
            session["navigationTarget"] = target;
        }
        session
    }

    pub fn live_session(&self, now: i64, subagents_at: Option<i64>) -> Value {
        let mut session = self.session(now, subagents_at);
        if let Some(facts) = self.live.facts(&self.id, &self.source, now, subagents_at) {
            session["liveFacts"] = facts;
        }
        session
    }
}
