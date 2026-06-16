//! Services: the use-cases the domain *offers* to the outside world.
//!
//! Each service is a concrete struct the API layer calls — the inbound side needs
//! no trait abstraction, since there is only ever one business-logic
//! implementation. They orchestrate the [`ports`](crate::ports) (the *outbound*
//! traits) to do their work and own what the entities deliberately do not:
//! authorization, the clock, and credential checks.

pub mod admin;
pub mod auth;
pub mod meta;
pub mod report;
pub mod sync;
