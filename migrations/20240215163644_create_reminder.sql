CREATE TABLE reminders ( 
    id SERIAL PRIMARY KEY,
    user_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    message_content TEXT NOT NULL,
    trigger_time TIMESTAMP NOT NULL,
    channel_id TEXT NOT NULL
);