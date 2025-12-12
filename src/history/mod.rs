use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct UndoHistory {
    past: Vec<String>,
    future: Vec<String>,
    current: Option<String>,
    last_snapshot_time: Instant,
    debounce_duration: Duration,
}

impl UndoHistory {
    pub fn new(initial: String) -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            current: Some(initial),
            last_snapshot_time: Instant::now(),
            debounce_duration: Duration::from_millis(500),
        }
    }

    pub fn new_empty() -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            current: None,
            last_snapshot_time: Instant::now(),
            debounce_duration: Duration::from_millis(500),
        }
    }

    pub fn push(&mut self, new_state: String) {
        // Bug: first keyword can't be undo
        if self.current.as_ref() == Some(&new_state) {
            return;
        }

        let now = Instant::now();
        let time_since_last = now.duration_since(self.last_snapshot_time);

        // If enough time passed or past is empty, save current to past
        if time_since_last >= self.debounce_duration || self.past.is_empty() {
            if let Some(current) = &self.current {
                self.past.push(current.clone());
                self.last_snapshot_time = now;
            }
        }

        self.current = Some(new_state);
        self.future.clear();
    }

    pub fn undo(&mut self) -> Option<String> {
        if let Some(prev) = self.past.pop() {
            self.future.push(self.current.clone().unwrap_or_default());
            self.current = Some(prev.clone());
            Some(prev)
        } else {
            None
        }
    }

    pub fn redo(&mut self) -> Option<String> {
        if let Some(next) = self.future.pop() {
            self.past.push(self.current.clone().unwrap_or_default());
            self.current = Some(next.clone());
            Some(next)
        } else {
            None
        }
    }

    pub fn current(&self) -> &Option<String> {
        &self.current
    }
}

