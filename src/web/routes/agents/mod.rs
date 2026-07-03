mod index;
mod show;
mod transactions;
mod memories;
mod prompts;
mod settings;
mod jobs;
mod hooks;
mod runs;
mod live_stream;
mod shared;
#[cfg(test)]
mod index_tests;
#[cfg(test)]
mod show_tests;
#[cfg(test)]
mod transactions_tests;
#[cfg(test)]
mod memories_tests;
#[cfg(test)]
mod prompts_tests;
#[cfg(test)]
mod settings_tests;
#[cfg(test)]
mod jobs_tests;
#[cfg(test)]
mod hooks_tests;
#[cfg(test)]
mod runs_tests;
#[cfg(test)]
mod live_stream_tests;
#[cfg(test)]
mod router_tests;

pub(in crate::web::routes) use self::{
    index::*, show::*, transactions::*, memories::*, prompts::*, settings::*,
    jobs::*, hooks::*, runs::*, live_stream::*,
};
use self::shared::*;
