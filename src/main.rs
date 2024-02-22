use std::time::Duration;
use std::{env, sync::Arc};

use clokwerk::{AsyncScheduler, TimeUnits};
use dotenv;
use regex::Regex;

use chrono::{Local, NaiveDateTime};

use serenity::all::{ChannelId, MessageId, UserId};
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;
use serenity::utils::MessageBuilder;
use serenity::{async_trait, http::Http};
use sqlx::{FromRow, PgPool};

struct Handler {
    pool: PgPool,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.content == "!help" {
            let help_message = MessageBuilder::new()
        .push("I can remind you about something in the future. ")
        .push("To set a reminder, use the `!remindme` command followed by a date and time. ")
        .push("For example, `!remindme 2021-01-01-12-00` or `!remindme 1d` ")
        .push("You can also add a message to the reminder, like this: `!remindme 2021-01-01-12-00 don't forget to call mom`")
        .build();
            let _ = msg.channel_id.say(&ctx.http, &help_message).await;
            return;
        }
        if msg.author.bot {
            return;
        }

        if let Some((date_str, text)) = parse_reminder_command(&msg.content) {
            if let Some(trigger_time) = parse_date_str(&date_str) {
                println!("Setting reminder for {:?}", trigger_time);
                let reminder = Reminder {
                    id: None,
                    user_id: msg.author.id.to_string(),
                    channel_id: msg.channel_id.to_string(),
                    message_id: msg.id.to_string(),
                    message_content: text.unwrap_or_else(|| "".to_string()),
                    trigger_time,
                };

                let result = sqlx::query!(
                    r#"
                    INSERT INTO reminders (user_id, message_id, message_content, trigger_time, channel_id)
                    VALUES ($1, $2, $3, $4, $5)
                    "#,
                    reminder.user_id,
                    reminder.message_id,
                    reminder.message_content,
                    reminder.trigger_time,
                    reminder.channel_id
                )
                .execute(&self.pool)
                .await;

                match result {
                    Ok(_) => {
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, "Reminder set successfully")
                            .await;
                    }
                    Err(e) => {
                        println!("Error setting reminder: {:?}", e);
                    }
                }
            } else {
                let _ = msg.channel_id.say(&ctx.http, "Invalid date format").await;
            }
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }
}

#[derive(Debug, FromRow)]
struct Reminder {
    id: Option<i32>,
    user_id: String,
    message_id: String,
    message_content: String,
    trigger_time: NaiveDateTime,
    channel_id: String,
}

async fn get_due_reminders(pool: &PgPool) -> Result<Vec<Reminder>, sqlx::Error> {
    let now = chrono::Local::now().naive_local();
    let reminders = sqlx::query_as!(
        Reminder,
        r#"SELECT * FROM reminders WHERE trigger_time < $1"#,
        now
    )
    .fetch_all(pool)
    .await?;
    Ok(reminders)
}

fn parse_reminder_command(message: &str) -> Option<(String, Option<String>)> {
    let regex = Regex::new(r"!remindme\s+(\S+)(?:\s+(.+))?").unwrap();

    regex.captures(message).map(|caps| {
        let date_str = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let text = caps.get(2).map(|m| m.as_str().to_string());
        (date_str, text)
    })
}

fn parse_date_str(date_str: &str) -> Option<NaiveDateTime> {
    let datetime_regex = Regex::new(r"^(\d{4})-(\d{2})-(\d{2})-(\d{2})-(\d{2})$").unwrap();
    let duration_regex = Regex::new(r"^(\d+)([mhdy])$").unwrap();

    if let Some(caps) = datetime_regex.captures(date_str) {
        let year = caps.get(1)?.as_str().parse::<i32>().ok()?;
        let month = caps.get(2)?.as_str().parse::<u32>().ok()?;
        let day = caps.get(3)?.as_str().parse::<u32>().ok()?;
        let hour = caps.get(4)?.as_str().parse::<u32>().ok()?;
        let minute = caps.get(5)?.as_str().parse::<u32>().ok()?;

        NaiveDateTime::parse_from_str(
            &format!("{}-{}-{} {}:{}:00", year, month, day, hour, minute),
            "%Y-%m-%d %H:%M:%S",
        )
        .ok()
    } else if let Some(caps) = duration_regex.captures(date_str) {
        let amount = caps.get(1)?.as_str().parse::<i64>().ok()?;
        let unit = caps.get(2)?.as_str();

        let duration = match unit {
            "m" => chrono::Duration::minutes(amount),
            "h" => chrono::Duration::hours(amount),
            "d" => chrono::Duration::days(amount),
            "y" => chrono::Duration::days(amount * 365),
            _ => return None,
        };

        let future_time = Local::now() + duration;
        Some(future_time.naive_local())
    } else {
        None
    }
}

async fn send_reminder(http: Arc<Http>, reminder: Reminder) {
    let user_id = reminder.user_id.parse::<UserId>().unwrap();
    let channel_id = reminder.channel_id.parse::<ChannelId>().unwrap();
    let message_id = reminder.message_id.parse::<MessageId>().unwrap();

    let user = http.get_user(user_id).await.unwrap();
    let channel = http.get_channel(channel_id).await.unwrap().guild().unwrap();
    let message = channel.message(http.as_ref(), message_id).await.unwrap();

    let reminder_response = MessageBuilder::new()
        .push("Hey ")
        .mention(&user)
        .push(", you asked me to remind you about this: ")
        .push(reminder.message_content)
        .push(" ")
        .push("reference message: ")
        .push(message.link())
        .build();

    if let Err(e) = message
        .channel_id
        .say(http.as_ref(), &reminder_response)
        .await
    {
        println!("Error sending reminder: {:?}", e);
    }
}

async fn cleanup_reminders(pool: &PgPool) {
    let now = chrono::Local::now().naive_local();
    let result = sqlx::query!(r#"DELETE FROM reminders WHERE trigger_time < $1"#, now)
        .execute(pool)
        .await;

    match result {
        Ok(_) => {}
        Err(e) => {
            println!("Error cleaning up reminders: {:?}", e);
        }
    }
}

async fn check_reminders_job(pool: PgPool, http: Arc<Http>) {
    println!("Checking reminders");
    let reminders = match get_due_reminders(&pool).await {
        Ok(reminders) => reminders,
        Err(e) => {
            println!("Error getting reminders: {:?}", e);
            return;
        }
    };

    for reminder in reminders {
        send_reminder(http.clone(), reminder).await;
    }
}

async fn cleanup_reminders_job(pool: PgPool) {
    println!("Cleaning up reminders");
    cleanup_reminders(&pool).await;
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment");
    let db_url = env::var("DATABASE_URL").expect("Expected a database URL in the environment");

    let pool = PgPool::connect(&db_url)
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!()
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let mut scheduler = AsyncScheduler::new();

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let bot = Handler { pool: pool.clone() };
    let http = Arc::new(Http::new(&token));
    let pool2 = pool.clone();

    scheduler.every(1.minutes()).run(move || {
        let pool = pool2.clone();
        let http = http.clone();

        async move {
            check_reminders_job(pool, http).await;
        }
    });

    scheduler.every(1.minutes()).run(move || {
        let pool = pool.clone();
        async move {
            cleanup_reminders_job(pool).await;
        }
    });

    tokio::spawn(async move {
        loop {
            scheduler.run_pending().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    let mut client = Client::builder(&token, intents)
        .event_handler(bot)
        .await
        .expect("Error creating client");

    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }
}
