use anyhow::{Context, Result};
use teloxide::{
    prelude::*,
    types::{AllowedUpdate, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode, User},
    utils::command::BotCommands,
};

use crate::gateway::model::TelegramGatewayConfig;

/// Telegram slash commands accepted by the gateway bot.
#[derive(BotCommands, Clone, Debug)]
#[command(
    rename_rule = "lowercase",
    parse_with = "split",
    description = "HyperVibes commands:"
)]
pub enum GatewayCommand {
    #[command(description = "start a new conversation")]
    New,
    #[command(description = "list and switch conversations")]
    Switch,
    #[command(description = "compact the current conversation")]
    Compact,
    #[command(description = "stop the active turn")]
    Stop,
    #[command(description = "set model (or create a chat): provider model")]
    Model(String, String),
    #[command(description = "show this help")]
    Help,
}

/// Inline keyboard callback prefix used for permission approvals.
pub const CALLBACK_PERMISSION: &str = "perm";
/// Inline keyboard callback prefix used for conversation switching.
pub const CALLBACK_SWITCH: &str = "switch";

/// Build a `teloxide::Bot` from a decrypted token. Caller must decrypt the
/// token first; this module never sees ciphertext.
pub fn build_bot(decrypted_token: &str) -> Bot {
    Bot::new(decrypted_token.to_string())
}

/// Fetch the bot's own username via `getMe`. Used after a token is stored so
/// the operator can render deep-link URLs without re-fetching.
pub async fn fetch_bot_username(bot: &Bot) -> Result<String> {
    let me = bot.get_me().send().await.context("getMe failed")?;
    Ok(me.username().to_string())
}

/// Send a permission approval prompt as an inline keyboard message. Returns
/// the sent message id so the service can edit it after the operator taps a
/// button.
pub async fn send_approval_keyboard(
    bot: &Bot,
    chat_id: ChatId,
    request_id: &str,
    permission: &str,
) -> Result<Message> {
    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            "Approve once",
            format!("{CALLBACK_PERMISSION}:{request_id}:once"),
        ),
        InlineKeyboardButton::callback(
            "Reject",
            format!("{CALLBACK_PERMISSION}:{request_id}:reject"),
        ),
    ]]);
    let text = format!("Permission required: `{permission}`");
    let message = bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await
        .context("failed to send approval keyboard")?;
    Ok(message)
}

/// Edit a previously sent permission message to show the operator's decision.
pub async fn edit_permission_message(
    bot: &Bot,
    chat_id: ChatId,
    message_id: teloxide::types::MessageId,
    decision: &str,
) -> Result<()> {
    bot.edit_message_text(chat_id, message_id, format!("Permission {decision}"))
        .await
        .context("failed to edit permission message")?;
    Ok(())
}

/// Send a `/switch` keyboard listing the operator's conversations. The
/// callback data encodes the conversation id.
pub async fn send_conversation_switcher(
    bot: &Bot,
    chat_id: ChatId,
    conversations: &[(uuid::Uuid, String)],
) -> Result<()> {
    if conversations.is_empty() {
        bot.send_message(chat_id, "No conversations yet. Use /new to start one.")
            .await
            .context("failed to send empty switcher")?;
        return Ok(());
    }
    let keyboard = InlineKeyboardMarkup::new(
        conversations
            .iter()
            .map(|(id, title)| {
                vec![InlineKeyboardButton::callback(
                    title.clone(),
                    format!("{CALLBACK_SWITCH}:{id}"),
                )]
            })
            .collect::<Vec<_>>(),
    );
    bot.send_message(chat_id, "Select a conversation:")
        .reply_markup(keyboard)
        .await
        .context("failed to send conversation switcher")?;
    Ok(())
}

/// Allow only message + callback_query updates when polling.
pub fn allowed_updates() -> Vec<AllowedUpdate> {
    vec![AllowedUpdate::Message, AllowedUpdate::CallbackQuery]
}

/// Extract a `/start <token>` payload from a Telegram message. Returns
/// `Some(token)` only when the text starts with `/start` and a single UUID
/// follows.
pub fn extract_start_payload(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let rest = trimmed.strip_prefix("/start")?.trim_start();
    if rest.is_empty() {
        return None;
    }
    let token = rest.split_whitespace().next()?;
    Some(token.to_string())
}

/// Try to parse an incoming text message as a gateway command. Returns
/// `Ok(Some(cmd))` when the message begins with a known slash command,
/// `Ok(None)` for ordinary chat text, and `Err` when the message looks like a
/// command but the arguments are malformed.
pub fn parse_command(text: &str, bot_username: &str) -> Result<Option<GatewayCommand>> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return Ok(None);
    }
    match GatewayCommand::parse(trimmed, bot_username) {
        Ok(cmd) => Ok(Some(cmd)),
        Err(teloxide::utils::command::ParseError::UnknownCommand(_)) => Ok(None),
        Err(error) => Err(anyhow::anyhow!(format!("{error:?}"))),
    }
}

/// Convert a [`User`] into a display name suitable for the pending link
/// record. Prefers the `@username`, then "First Last", then "First", then
/// the numeric user id.
pub fn user_display_name(user: &User) -> String {
    if let Some(username) = user.username.as_deref() {
        return format!("@{username}");
    }
    let first = user.first_name.as_str();
    if let Some(last) = user.last_name.as_deref() {
        return format!("{first} {last}");
    }
    first.to_string()
}

/// Determine whether a Telegram chat id is authorized to send messages to
/// the bot for a given gateway config.
pub fn is_authorized_chat(config: &TelegramGatewayConfig, chat_id: i64) -> bool {
    config.chat_id.is_some_and(|stored| stored == chat_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use teloxide::types::UserId;

    fn user(username: Option<&str>, first: &str, last: Option<&str>) -> User {
        User {
            id: UserId(1),
            is_bot: false,
            first_name: first.to_string(),
            last_name: last.map(ToString::to_string),
            username: username.map(ToString::to_string),
            language_code: None,
            is_premium: false,
            added_to_attachment_menu: false,
        }
    }

    #[test]
    fn extract_start_payload_returns_token_when_present() {
        assert_eq!(
            extract_start_payload("/start 12345678-1234-1234-1234-1234567890ab"),
            Some("12345678-1234-1234-1234-1234567890ab".to_string())
        );
    }

    #[test]
    fn extract_start_payload_returns_none_for_plain_start() {
        assert_eq!(extract_start_payload("/start"), None);
        assert_eq!(extract_start_payload("/start "), None);
    }

    #[test]
    fn extract_start_payload_returns_none_for_other_text() {
        assert_eq!(extract_start_payload("hello"), None);
        assert_eq!(extract_start_payload("/help"), None);
    }

    #[test]
    fn parse_command_returns_none_for_plain_text() {
        assert!(parse_command("hello world", "mybot").unwrap().is_none());
    }

    #[test]
    fn parse_command_returns_none_for_unknown_command() {
        assert!(parse_command("/unknown", "mybot").unwrap().is_none());
    }

    #[test]
    fn parse_command_parses_help_command() {
        let cmd = parse_command("/help", "mybot").unwrap().unwrap();
        assert!(matches!(cmd, GatewayCommand::Help));
    }

    #[test]
    fn parse_command_parses_model_command_with_args() {
        let cmd = parse_command("/model anthropic claude-sonnet-4", "mybot")
            .unwrap()
            .unwrap();
        match cmd {
            GatewayCommand::Model(provider, model) => {
                assert_eq!(provider, "anthropic");
                assert_eq!(model, "claude-sonnet-4");
            }
            other => panic!("expected Model, got {other:?}"),
        }
    }

    #[test]
    fn user_display_name_prefers_username() {
        let u = user(Some("alice"), "Alice", Some("Smith"));
        assert_eq!(user_display_name(&u), "@alice");
    }

    #[test]
    fn user_display_name_falls_back_to_first_last() {
        let u = user(None, "Alice", Some("Smith"));
        assert_eq!(user_display_name(&u), "Alice Smith");
    }

    #[test]
    fn user_display_name_falls_back_to_first_only() {
        let u = user(None, "Alice", None);
        assert_eq!(user_display_name(&u), "Alice");
    }

    #[test]
    fn is_authorized_chat_only_accepts_stored_chat_id() {
        let config = TelegramGatewayConfig {
            chat_id: Some(42),
            ..Default::default()
        };
        assert!(is_authorized_chat(&config, 42));
        assert!(!is_authorized_chat(&config, 43));
    }

    #[test]
    fn is_authorized_chat_rejects_when_chat_not_bound() {
        let config = TelegramGatewayConfig::default();
        assert!(!is_authorized_chat(&config, 42));
    }
}
