mod database;

use teloxide::utils::command::BotCommands;
use std::error::Error;
use lazy_static::lazy_static;
use std::path::Path;
use teloxide::dispatching::dialogue::InMemStorage;
use teloxide::prelude::*;
use teloxide::types::{BotCommand, InlineKeyboardButton, InlineKeyboardMarkup, InlineQueryResult, InlineQueryResultArticle, InputMessageContent, InputMessageContentText, Me, MessageId};
use crate::database::{Database, User, DBMessage};

lazy_static! {
    static ref DATABASE: Database = Database::new(Path::new("database.sqlite")).unwrap_or_else(|error| {
        panic!("Failed to initialize database connection: {}", error);
    });
}

#[derive(BotCommands)]
#[command(rename_rule = "lowercase")]
enum Command {
    /// Start
    CreatePoll,
}

#[derive(Clone, Default)]
pub enum State {
    #[default]
    Start,
    ReceiveDescription,
    ReceiveAnswerOption {
        description: String
    }
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    log::info!("Starting fluffy bot :3");

    let bot = init_bot().await;
    //bot.log_out().await?;

    let handler = dptree::entry()
        .branch(
            Update::filter_message().enter_dialogue::<Message, InMemStorage<State>, State>()
                .branch(dptree::case![State::Start].endpoint(start_poll_creation))
                .branch(dptree::case![State::ReceiveDescription].endpoint(receive_description))

        )
        .branch(Update::filter_callback_query().endpoint(handle_votes))
        .branch(Update::filter_inline_query().endpoint(handle_share_poll))
        .branch(Update::filter_chosen_inline_result().endpoint(handle_chosen_share_poll));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![InMemStorage::<State>::new()])
        .enable_ctrlc_handler().build()
        .dispatch()
        .await;
}

async fn init_bot() -> Bot {
    let bot = Bot::from_env();
    let commands = vec! [
        BotCommand::new("createpoll", "Create a new poll"),
    ];
    bot.set_my_commands(commands).await.expect("Failed to set commands");
    bot
}

type BotResult = Result<(), Box<dyn Error + Send + Sync>>;
type BotDialogue = Dialogue<State, InMemStorage<State>>;

async fn start_poll_creation(bot: Bot, me: Me, dialogue: BotDialogue, message: Message) -> BotResult {
    if let Some(text) = message.text() {
        match BotCommands::parse(text, me.username()) {
            Ok(Command::CreatePoll) => {
                bot.send_message(message.chat.id, "Send me the description of the new poll!").await?;
                dialogue.update(State::ReceiveDescription).await?;
            }
            Err(_) => {
                if text.starts_with("/") {
                    bot.send_message(message.chat.id, "Command not found!").await?;
                    return Ok(());
                }
                bot.send_message(message.chat.id, "Please use a command to start interaction with me!").await?;
            }
        }
    } else {
        bot.send_message(message.chat.id, "Please only use text to communicate with me!").await?;
    }
    Ok(())
}

async fn receive_description(bot: Bot, dialogue: BotDialogue, message: Message) -> BotResult {
    if let Some(description) = message.text() {
        if let Some(user) = message.from() {
            let posted_message = bot.send_message(message.chat.id, format!("{}\n\n Anmeldungen: 0", description)).reply_markup(create_poll_buttons(true)).await?;
            DATABASE.add_poll_and_link_message(user.id.to_string(), description.to_string(), posted_message.chat.id.to_string(), posted_message.id.to_string())?;
            dialogue.exit().await?;
        } else {
            bot.send_message(message.chat.id, "Cannot link to user, are you using a channel to write me? If yes switch to your user").await?;
        }
    } else {
        bot.send_message(message.chat.id, "You can only have text as a description. try again!").await?;
    }
    Ok(())
}

#[derive(strum_macros::Display)]
enum InlineButtonType {
    Vote,
    Close
}

fn create_poll_buttons(create_close_button: bool) -> InlineKeyboardMarkup {
    let mut buttons:Vec<Vec<InlineKeyboardButton>> = Vec::new();
    buttons.push(vec![InlineKeyboardButton::callback("Ja, ich komme!".to_owned(), InlineButtonType::Vote.to_string())]);
    if create_close_button {
        buttons.push(vec![InlineKeyboardButton::callback("Poll beenden!".to_owned(), InlineButtonType::Close.to_string())]);
    }
    InlineKeyboardMarkup::new(buttons)
}

async fn handle_votes(bot: Bot, q: CallbackQuery) -> BotResult {
    let callback_data = q.data.unwrap_or(String::from(""));
    let is_close = callback_data == InlineButtonType::Close.to_string();
    // TODO: reduce this mess
    if let Some(message) = q.message {
        let mut poll = DATABASE.get_poll_id(message.id.to_string(), message.chat.id.to_string().into())?;
        if is_close {
            poll.is_active = DATABASE.change_poll_is_active(poll.id)?;
        }
        if poll.is_active == 0 && !is_close {
            return Ok(())
        }
        if !is_close {
            DATABASE.add_vote_for_poll_message(q.from.username.unwrap_or(String::from("")), message.id.to_string(), message.chat.id.to_string().into())?;
        }
        let messages_to_update= DATABASE.get_messages_for_poll(poll.id)?;
        let votes = DATABASE.get_votes_for_poll(message.id.to_string(), message.chat.id.to_string().into())?;
        update_messages(bot.clone(), messages_to_update, votes, poll.description, poll.is_active).await?;
    } else if let Some(inline_message_id) = q.inline_message_id {
        let poll = DATABASE.get_poll_id(inline_message_id.clone(), None)?;
        if callback_data == InlineButtonType::Close.to_string() {
            DATABASE.change_poll_is_active(poll.id)?;
        }
        if poll.is_active == 0 && !is_close {
            return Ok(())
        }
        if !is_close {
            DATABASE.add_vote_for_poll_message(q.from.username.unwrap_or(String::from("")), inline_message_id.clone(), None)?;
        }
        let messages_to_update= DATABASE.get_messages_for_poll(poll.id)?;
        let votes = DATABASE.get_votes_for_poll(inline_message_id.clone(), None)?;
        update_messages(bot.clone(), messages_to_update, votes, poll.description, poll.is_active).await?;
    }
    bot.answer_callback_query(q.id).await?;
    Ok(())
}

async fn update_messages(bot: Bot, messages: Vec<DBMessage>, votes:Vec<User>, description: String, is_active: u32) -> BotResult {
    if messages.len() == 0 {
        return Ok(());
    }
    let mut updated_text:String = String::from(&format!("{}\n\nAnmeldungen: {}\n", description, votes.len()));
    for vote in votes {
        if vote.username != "" {
            updated_text.push_str(&format!("@{}\n", vote.username));
        }
    }
    if is_active == 0 {
        updated_text = updated_text + "\n\n Die Umfrage wurde beendet!";
    }

    for message in messages {
        if message.chat_id == "" {
            let mut request = bot.edit_message_text_inline(message.message_id.to_string(), updated_text.to_string());
            if is_active == 1 {
                request = request.reply_markup(create_poll_buttons(false));
            }
            request.await?;
        } else {
            let message_id:i32 = message.message_id.parse().unwrap();
            bot.edit_message_text(message.chat_id, MessageId(message_id), updated_text.to_string()).reply_markup(create_poll_buttons(true)).await?;
        }
    }
    Ok(())
}

async fn handle_share_poll(bot: Bot, q:InlineQuery) -> BotResult {
    println!("{}", q.query);
    let polls = DATABASE.find_active_polls(q.from.id.to_string())?;
    if polls.len() == 0 {
        return Ok(());
    }

    let mut poll_selection:Vec<InlineQueryResult> = vec![];
    for poll in polls {
        let single_poll = InlineQueryResultArticle::new(
            poll.id.to_string(),
            poll.description.clone(),
            InputMessageContent::Text(InputMessageContentText::new(format!("{}\n\nAnmeldungen: 0\n", poll.description)))
        ).reply_markup(create_poll_buttons(false));
        poll_selection.push(single_poll.into());
    }
    bot.answer_inline_query(q.id, poll_selection).await?;
    Ok(())
}

async fn handle_chosen_share_poll(q:ChosenInlineResult) -> BotResult {
    DATABASE.link_inline_message_to_poll(q.inline_message_id.unwrap(),
                                         q.result_id.parse()?)?;
    Ok(())
}