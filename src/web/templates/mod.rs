mod account;
mod agents;
mod balance;
mod conversations;
mod live_health;
mod memories;
mod navbar;
mod opencode;
mod orders;
mod positions;
mod providers;
mod runs;
mod settings;
pub(crate) mod shared;
mod sub_agents;
mod workspace;

#[cfg(test)]
use self::shared::*;

pub use account::*;
pub use agents::*;
pub use balance::*;
pub use conversations::*;
pub use live_health::*;
pub use memories::*;
pub use navbar::*;
pub use opencode::*;
pub use orders::*;
pub use positions::*;
pub use providers::*;
pub use runs::*;
pub use settings::*;
pub use sub_agents::*;
pub use workspace::*;

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "shared_tests.rs"]
mod shared_tests;

#[cfg(test)]
#[path = "agents_tests.rs"]
mod agents_tests;

#[cfg(test)]
#[path = "opencode_tests.rs"]
mod opencode_tests;

#[cfg(test)]
#[path = "sub_agents_tests.rs"]
mod sub_agents_tests;

#[cfg(test)]
#[path = "runs_tests.rs"]
mod runs_tests;

#[cfg(test)]
#[path = "conversations_tests.rs"]
mod conversations_tests;

#[cfg(test)]
#[path = "balance_tests.rs"]
mod balance_tests;

#[cfg(test)]
#[path = "positions_tests.rs"]
mod positions_tests;

#[cfg(test)]
#[path = "orders_tests.rs"]
mod orders_tests;

#[cfg(test)]
#[path = "memories_tests.rs"]
mod memories_tests;

#[cfg(test)]
#[path = "settings_tests.rs"]
mod settings_tests;
