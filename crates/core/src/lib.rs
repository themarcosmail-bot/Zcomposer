use std::{path::Path, time::Duration};

use rusqlite::{Connection, TransactionBehavior, params};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Queued,
    Running,
    Completed,
    Failed,
    Interrupted,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    fn predecessors(self) -> &'static [&'static str] {
        match self {
            Self::Queued => &[],
            Self::Running => &["queued"],
            Self::Completed | Self::Failed => &["running"],
            Self::Interrupted => &["queued", "running"],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Session {
    pub id: i64,
    pub agent: String,
    pub task: String,
    pub state: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Event {
    pub cursor: i64,
    pub session_id: i64,
    pub state: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub cursor: i64,
    pub sessions: Vec<Session>,
}

pub struct Store(Connection);

impl Store {
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS sessions (
                 id INTEGER PRIMARY KEY,
                 agent TEXT NOT NULL,
                 task TEXT NOT NULL,
                 state TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS events (
                 cursor INTEGER PRIMARY KEY,
                 session_id INTEGER NOT NULL REFERENCES sessions(id),
                 state TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS turn_results (
                 session_id INTEGER PRIMARY KEY REFERENCES sessions(id),
                 thread_id TEXT NOT NULL,
                 answer TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS studio_layout (
                 session_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                 position INTEGER NOT NULL UNIQUE
             );
             COMMIT;",
        )?;
        Ok(Self(connection))
    }

    pub fn create(&mut self, agent: &str, task: &str) -> rusqlite::Result<i64> {
        let transaction = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = State::Queued.as_str();
        transaction.execute(
            "INSERT INTO sessions (agent, task, state) VALUES (?1, ?2, ?3)",
            params![agent, task, state],
        )?;
        let id = transaction.last_insert_rowid();
        transaction.execute(
            "INSERT INTO events (session_id, state) VALUES (?1, ?2)",
            params![id, state],
        )?;
        transaction.commit()?;
        Ok(id)
    }

    /// Returns false when the session is missing or the transition is no longer valid.
    pub fn transition(&mut self, id: i64, next: State) -> rusqlite::Result<bool> {
        self.update(id, next, None)
    }

    pub fn finish_turn(
        &mut self,
        id: i64,
        thread_id: &str,
        answer: &str,
    ) -> rusqlite::Result<bool> {
        self.update(id, State::Completed, Some((thread_id, answer)))
    }

    pub fn answer(&self, id: i64) -> rusqlite::Result<String> {
        self.0.query_row(
            "SELECT answer FROM turn_results WHERE session_id = ?1",
            [id],
            |row| row.get(0),
        )
    }

    fn update(
        &mut self,
        id: i64,
        next: State,
        result: Option<(&str, &str)>,
    ) -> rusqlite::Result<bool> {
        let transaction = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let predecessors = next.predecessors();
        let previous: String =
            match transaction.query_row("SELECT state FROM sessions WHERE id = ?1", [id], |row| {
                row.get(0)
            }) {
                Ok(state) => state,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
                Err(error) => return Err(error),
            };
        if !predecessors.contains(&previous.as_str()) {
            return Ok(false);
        }
        let state = next.as_str();
        transaction.execute(
            "UPDATE sessions SET state = ?1 WHERE id = ?2",
            params![state, id],
        )?;
        transaction.execute(
            "INSERT INTO events (session_id, state) VALUES (?1, ?2)",
            params![id, state],
        )?;
        if let Some((thread_id, answer)) = result {
            transaction.execute(
                "INSERT INTO turn_results (session_id, thread_id, answer) VALUES (?1, ?2, ?3)",
                params![id, thread_id, answer],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn snapshot(&mut self) -> rusqlite::Result<Snapshot> {
        let transaction = self.0.transaction()?;
        let cursor =
            transaction.query_row("SELECT COALESCE(MAX(cursor), 0) FROM events", [], |row| {
                row.get(0)
            })?;
        let sessions = {
            let mut query =
                transaction.prepare("SELECT id, agent, task, state FROM sessions ORDER BY id")?;
            query
                .query_map([], |row| {
                    Ok(Session {
                        id: row.get(0)?,
                        agent: row.get(1)?,
                        task: row.get(2)?,
                        state: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?
        };
        transaction.commit()?;
        Ok(Snapshot { cursor, sessions })
    }

    pub fn events_after(&self, cursor: i64) -> rusqlite::Result<Vec<Event>> {
        let mut query = self.0.prepare(
            "SELECT cursor, session_id, state FROM events WHERE cursor > ?1 ORDER BY cursor LIMIT 100",
        )?;
        query
            .query_map([cursor], |row| {
                Ok(Event {
                    cursor: row.get(0)?,
                    session_id: row.get(1)?,
                    state: row.get(2)?,
                })
            })?
            .collect()
    }

    pub fn session_history(&self, id: i64) -> rusqlite::Result<Vec<String>> {
        let mut query = self
            .0
            .prepare("SELECT state FROM events WHERE session_id = ?1 ORDER BY cursor")?;
        query.query_map([id], |row| row.get(0))?.collect()
    }

    pub fn studio_order(&self) -> rusqlite::Result<Vec<i64>> {
        let mut query = self
            .0
            .prepare("SELECT session_id FROM studio_layout ORDER BY position")?;
        query.query_map([], |row| row.get(0))?.collect()
    }

    pub fn save_studio_order(&mut self, ids: &[i64]) -> rusqlite::Result<()> {
        let transaction = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM studio_layout", [])?;
        {
            let mut insert = transaction
                .prepare("INSERT INTO studio_layout (session_id, position) VALUES (?1, ?2)")?;
            for (position, id) in ids.iter().enumerate() {
                insert.execute(params![id, position])?;
            }
        }
        transaction.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_transition_leaves_snapshot_and_history_unchanged() -> rusqlite::Result<()> {
        let mut store = Store::open(":memory:")?;
        let id = store.create("codex", "Revisar exemplo")?;
        assert!(!store.transition(id, State::Completed)?);
        assert!(!store.transition(id + 1, State::Running)?);
        assert!(store.transition(id, State::Running)?);
        assert!(store.transition(id, State::Interrupted)?);
        let snapshot = store.snapshot()?;
        assert!(!store.transition(id, State::Completed)?);
        assert_eq!(store.snapshot()?, snapshot);
        assert!(store.events_after(snapshot.cursor)?.is_empty());
        let history = store.events_after(0)?;
        let states: Vec<_> = history.iter().map(|event| event.state.as_str()).collect();
        assert_eq!(states, ["queued", "running", "interrupted"]);
        Ok(())
    }

    #[test]
    fn completed_turn_keeps_its_first_answer() -> rusqlite::Result<()> {
        let mut store = Store::open(":memory:")?;
        let id = store.create("codex", "Responder OK")?;
        assert!(!store.finish_turn(id, "thread", "premature")?);
        assert!(store.transition(id, State::Running)?);
        assert!(store.finish_turn(id, "thread", "OK")?);
        assert!(!store.finish_turn(id, "thread", "duplicate")?);
        assert_eq!(store.answer(id)?, "OK");
        assert_eq!(store.events_after(2)?.len(), 1);
        Ok(())
    }

    #[test]
    fn invalid_layout_keeps_saved_order_and_session_history() -> rusqlite::Result<()> {
        let mut store = Store::open(":memory:")?;
        let first = store.create("codex", "Primeira tarefa")?;
        let second = store.create("codex", "Segunda tarefa")?;
        store.transition(first, State::Running)?;
        assert!(store.studio_order()?.is_empty());

        store.save_studio_order(&[second, first])?;
        assert!(store.save_studio_order(&[first, second, first]).is_err());
        assert_eq!(store.studio_order()?, [second, first]);
        assert_eq!(store.session_history(first)?, ["queued", "running"]);
        assert_eq!(store.session_history(second)?, ["queued"]);
        Ok(())
    }
}
