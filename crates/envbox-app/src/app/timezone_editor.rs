//! Lookup preview belongs to the current Profile edit session, never storage.
use super::EnvBoxApp;
use crate::message::{Message, Nav};
use crate::timezone_lookup::{self, LookupResult, Source};
use iced::Task;

impl EnvBoxApp {
    pub(super) fn query_timezone(&mut self) -> Task<Message> {
        if self.timezone_lookup.pending
            || self.nav != Nav::Profiles
            || !(self.profile_edit_mode || self.profile_draft.id.is_none())
        {
            return Task::none();
        }
        let source = self.timezone_lookup.source;
        let explicit = if source == Source::Explicit {
            match timezone_lookup::parse_public_ip(&self.timezone_lookup.input) {
                Ok(ip) => Some(ip),
                Err(error) => {
                    self.timezone_lookup.error = Some(error);
                    return Task::none();
                }
            }
        } else {
            None
        };
        self.timezone_lookup.invalidate();
        self.timezone_lookup.pending = true;
        let generation = self.timezone_lookup.generation;
        let (sender, receiver) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = sender.send(timezone_lookup::query(source, explicit));
        });
        Task::perform(
            async move {
                receiver
                    .await
                    .unwrap_or_else(|_| Err("查询失败：后台请求已结束，请重试".into()))
            },
            move |result| Message::TimezoneResult(generation, result),
        )
    }

    pub(super) fn finish_timezone_query(
        &mut self,
        generation: u64,
        result: Result<LookupResult, String>,
    ) {
        if generation != self.timezone_lookup.generation
            || !self.timezone_lookup.pending
            || self.nav != Nav::Profiles
            || !(self.profile_edit_mode || self.profile_draft.id.is_none())
        {
            return;
        }
        self.timezone_lookup.pending = false;
        match result {
            Ok(result) => self.timezone_lookup.result = Some(result),
            Err(error) => self.timezone_lookup.error = Some(error),
        }
    }

    pub(super) fn fill_timezone(&mut self) {
        if self.nav != Nav::Profiles || !(self.profile_edit_mode || self.profile_draft.id.is_none())
        {
            return;
        }
        let Some(result) = self.timezone_lookup.result.as_ref() else {
            return;
        };
        if !result.can_fill {
            return;
        }
        let Some(windows) = result.windows.as_ref() else {
            return;
        };
        self.profile_draft.tz = windows.clone();
        self.profile_draft.tz_iana = result.iana.clone();
        self.timezone_lookup.invalidate();
    }
}
