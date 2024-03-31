use std::path::Path;
use std::sync::Mutex;
use rusqlite::{Connection, Error};

pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn new(path: &Path) -> Result<Database, Error> {
        let connection = Connection::open(path)?;
        //let connection = Connection::open_in_memory()?; // used for testing
        connection.execute(
            "CREATE TABLE IF NOT EXISTS poll (
                    id              INTEGER PRIMARY KEY,
                    creator         TEXT,
                    description     TEXT NOT NULL,
                    is_active       INTEGER
        )",
            (),
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS message (
                    message_id      TEXT PRIMARY KEY,
                    poll_id         INTEGER,
                    FOREIGN KEY (poll_id) REFERENCES poll(id)
            )",
            (),
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS vote (
                    id              INTEGER PRIMARY KEY,
                    user_id         INTEGER,
                    poll_id         INTEGER,
                    FOREIGN KEY (user_id) REFERENCES user(id),
                    FOREIGN KEY (poll_id) REFERENCES poll(id)
        )",
            (),
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS user (
                    id              TEXT PRIMARY KEY,
                    name            TEXT
        )",
            (),
        )?;
        Ok(Database { connection: connection.into() })
    }

    pub fn add_poll(&self,
                    creator: String,
                    description: String) -> Result<(), Error> {
        self.connection.lock().unwrap().execute(
            "INSERT INTO poll (creator, description, is_active) VALUES (?1, ?2, ?3)",
            (&creator, &description, 1),
        )?;
        Ok(())
    }

    pub fn link_message_to_poll(&self,
                                message_id: String,
                                poll_id: String,
    ) -> Result<(), Error> {
        self.connection.lock().unwrap().execute(
            "INSERT INTO message (message_id, poll_id) VALUES (?1, ?2)",
            (&message_id, &poll_id),
        )?;
        Ok(())
    }

    pub fn add_vote_for_poll(&self,
                             user: u32,
                             poll: u32) -> Result<(), Error> {
        self.connection.lock().unwrap().execute(
            "INSERT INTO vote (user_id, poll_id) VALUES (?1, ?2)",
            (&user, &poll),
        )?;
        Ok(())
    }
}