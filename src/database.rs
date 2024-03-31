
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use rusqlite::{Connection, Error};

pub struct Database {
    connection: Mutex<Connection>,
}

pub struct Poll {
    pub id: u32,
    pub description: String,
    pub is_active: u32
}

pub struct DBMessage {
    pub chat_id: String,
    pub message_id:  String,
}

pub struct User {
    pub username: String
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
                    chat_id         TEXT,
                    message_id      TEXT,
                    poll_id         INTEGER,
                    FOREIGN KEY (poll_id) REFERENCES poll(id),
                    PRIMARY KEY (chat_id, message_id)
            )",
            (),
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS vote (
                    user_id         TEXT,
                    poll_id         INTEGER,
                    FOREIGN KEY (poll_id) REFERENCES poll(id),
                    PRIMARY KEY (user_id, poll_id)
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

    pub fn add_poll_and_link_message(&self,
                                     creator: String,
                                     description: String,
                                     chat_id: String,
                                     message_id: String,
    ) -> Result<(), Error> {
        let connection = self.connection.lock().unwrap();
        connection.execute(
            "INSERT INTO poll (creator, description, is_active) VALUES (?1, ?2, ?3)",
            (&creator, &description, 1),
        )?;
        self.link_message_to_poll(chat_id, message_id, connection.last_insert_rowid(), Some(connection))?;
        Ok(())
    }

    pub fn link_message_to_poll(&self,
                                chat_id: String,
                                message_id: String,
                                poll_id: i64,
                                connection: Option<MutexGuard<Connection>>,
    ) -> Result<(), Error> {
        let locked_connection: MutexGuard<Connection>;
        match connection {
            Some(connection) => {
                locked_connection = connection;
            }
            None => {
                locked_connection = self.connection.lock().unwrap();
            }
        }
        locked_connection.execute(
            "INSERT INTO message (chat_id, message_id, poll_id) VALUES (?1, ?2, ?3)",
            (&chat_id, &message_id, &poll_id),
        )?;
        Ok(())
    }

    pub fn link_inline_message_to_poll(&self,
                                       message_id: String,
                                       poll_id: u32,
    ) -> Result<(), Error> {
        self.connection.lock().unwrap().execute(
            "INSERT INTO message (chat_id, message_id, poll_id) VALUES ('', ?1, ?2)",
            (&message_id, &poll_id),
        )?;
        Ok(())
    }

    pub fn add_vote_for_poll_message(&self,
                                     user: String,
                                     message_id: String,
                                     chat_id: Option<String>
    ) -> Result<(), Error> {
        let poll: Poll;

        poll = self.get_poll_id(message_id, chat_id)?;
        if poll.is_active == 0 {
            // poll closed
            return Ok(());
        }

        let mut locked_connection = self.connection.lock().unwrap();
        // insert or remove vote
        let transaction = locked_connection.transaction()?;
        transaction.execute("DELETE FROM vote WHERE user_id = ?1 AND poll_id = ?2", &[&user, &poll.id.to_string()])?;
        let num_deleted = transaction.query_row("SELECT changes() AS num_deleted", [], |row| row.get::<_, i32>(0))?;
        if num_deleted == 0 {
            transaction.execute("INSERT INTO vote (user_id, poll_id) VALUES (?, ?)", &[&user, &poll.id.to_string()])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get_messages_for_poll(&self, poll_id: u32) -> Result<Vec<DBMessage>, Error> {
        // get all messages for poll so they get updated
        let locked_connection = self.connection.lock().unwrap();
        let mut messages = locked_connection.prepare(
            "SELECT chat_id, message_id FROM message WHERE poll_id = ?1"
        )?;
        let message_results = messages.query_map([poll_id], |row| {
            Ok(DBMessage {
                chat_id: row.get(0).unwrap_or("".to_string()),
                message_id: row.get(1).unwrap_or("".to_string()),
            })
        })?;

        let mut chat_messages = Vec::new();
        for result in message_results {
            chat_messages.push(
                result.unwrap_or(DBMessage {
                    chat_id: "".to_string(),
                    message_id: "".to_string()
                })
            );
        }
        Ok(chat_messages)
    }


    pub fn get_poll_id(&self,
                       message_id: String,
                       chat_id: Option<String>,
    ) -> Result<Poll, Error> {
        let sql;
        let mut chat_id_result = String::from("");
        if let Some(chat_id_text) = chat_id {
            chat_id_result = chat_id_text;
            sql = "SELECT id, description, is_active FROM poll WHERE id = (
                SELECT poll_id FROM message WHERE chat_id = ?1 AND message_id = ?2 )";
        } else {
            // message was sent with inline query
            sql = "SELECT id, description, is_active FROM poll WHERE id = (
                SELECT poll_id FROM message WHERE message_id = ?1 )"
        }
        let locked_connection = self.connection.lock().unwrap();
        let poll;
        {
            let mut get_polls = locked_connection.prepare(sql)?;
            if chat_id_result != "" {
                poll = get_polls.query_row([chat_id_result, message_id], |row| {
                    Ok(Poll {
                        id: row.get(0)?,
                        description: row.get(1)?,
                        is_active: row.get(2)?,
                    })
                })?;
            } else {
                poll = get_polls.query_row([message_id], |row| {
                    Ok(Poll {
                        id: row.get(0)?,
                        description: row.get(1)?,
                        is_active: row.get(2)?,
                    })
                })?;
            }
        }
        drop(locked_connection); // TODO: is this really a good idea?
        Ok(poll)
    }

    pub fn get_votes_for_poll(&self,
                              message_id: String,
                              chat_id: Option<String>
    ) -> Result<Vec<User>, Error> {
        let poll = self.get_poll_id(message_id, chat_id)?;
        let locked_connection = self.connection.lock().unwrap();
        let mut votes_query = locked_connection.prepare(
            "SELECT user_id FROM VOTE WHERE poll_id = ?1"
        )?;
        let votes = votes_query.query_map([poll.id], |row| {
            Ok(
                User {
                    username: row.get(0)?
                }
            )
        })?;

        let mut votes_result = Vec::new();
        for vote in votes {
            votes_result.push(
                vote.unwrap_or(User {
                    username: "".to_string(),
                })
            );
        }
        Ok(votes_result)
    }

    pub fn find_active_polls(&self, creator: String) -> Result<Vec<Poll>, Error> {
        let locked_connection = self.connection.lock().unwrap();
        let mut active_polls_statement = locked_connection.prepare(
            "SELECT id, description FROM poll WHERE creator = ?1 AND is_active = 1"
        )?;
        let polls = active_polls_statement.query_map([creator], |row| {
            Ok(
                Poll {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    is_active: 1,
                }
            )
        })?;

        let poll_result = polls.filter_map(Result::ok).collect();
        Ok(poll_result)
    }

    pub fn change_poll_is_active(&self, poll_id: u32) ->  Result<u32, Error>{
        let mut locked_connection = self.connection.lock().unwrap();
        let transaction = locked_connection.transaction()?;
        let is_active = transaction.query_row("SELECT is_active FROM poll WHERE id = ?1", [poll_id], |row| row.get::<_, i32>(0))?;
        let new_active = if is_active == 0 { 1 } else { 0 };
        transaction.execute("UPDATE poll SET is_active = ?1 WHERE id = ?2", [new_active, poll_id])?;
        transaction.commit()?;
        Ok(new_active)
    }
}