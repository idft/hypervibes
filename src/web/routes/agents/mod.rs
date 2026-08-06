mod index;
#[cfg(test)]
mod index_tests;
mod jobs;
#[cfg(test)]
mod jobs_tests;
mod live_stream;
#[cfg(test)]
mod live_stream_tests;
mod memories;
#[cfg(test)]
mod memories_tests;
mod operations;
mod prompts;
#[cfg(test)]
mod prompts_tests;
#[cfg(test)]
mod router_tests;
mod runs;
#[cfg(test)]
mod runs_tests;
mod settings;
#[cfg(test)]
mod settings_tests;
mod shared;
mod show;
#[cfg(test)]
mod show_tests;
mod transactions;
#[cfg(test)]
mod transactions_tests;

#[cfg(test)]
use self::shared::*;
pub(in crate::web::routes) use self::{
    chat::*, index::*, jobs::*, live_stream::*, memories::*, operations::*, prompts::*, runs::*,
    settings::*, show::*, transactions::*,
};
mod chat;
#[cfg(test)]
mod chat_tests;
