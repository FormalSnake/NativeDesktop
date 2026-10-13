use nd_core::NodeId;

use crate::events::GestureEvent;
use crate::input::{PointerInput, PointerPhase};
use crate::recognizer::{GestureKind, Recognizer, Verdict};

/// One recognizer on one node, as the backend's hit test found it.
pub struct ArenaMember {
    pub node: NodeId,
    pub recognizer: Box<dyn Recognizer>,
}

impl ArenaMember {
    pub fn new(node: NodeId, recognizer: impl Recognizer + 'static) -> Self {
        Self { node, recognizer: Box::new(recognizer) }
    }
}

struct Member {
    node: NodeId,
    recognizer: Box<dyn Recognizer>,
    rejected: bool,
    /// Events written while the arena was undecided.
    pending: Vec<GestureEvent>,
}

impl Member {
    fn kind(&self) -> GestureKind {
        self.recognizer.kind()
    }
}

/// Pointer gesture arbitration for one window. A press opens a session with the recognizers of
/// every node under it, innermost first. Every member sees every input; the first to accept wins
/// and the rest lose, ties going to the innermost; a member left alone wins by default. Events
/// reach the app only from the winner, so a tap inside a draggable card never fires once the drag
/// claimed the press, and the card never moves for a tap.
#[derive(Default)]
pub struct Arena {
    members: Vec<Member>,
    winner: Option<usize>,
    pressed: bool,
}

impl Arena {
    /// A press. `members` are the hit path's recognizers, innermost first. Members held from the
    /// previous press (a double tap's first tap) replace the matching fresh ones.
    pub fn down(&mut self, members: Vec<ArenaMember>, input: &PointerInput) -> Vec<(NodeId, GestureEvent)> {
        let mut held: Vec<Member> =
            std::mem::take(&mut self.members).into_iter().filter(|m| !m.rejected && !m.recognizer.finished()).collect();
        self.winner = None;
        self.pressed = true;
        self.members = members
            .into_iter()
            .map(|m| {
                let carried = held.iter().position(|h| h.node == m.node && h.kind() == m.recognizer.kind());
                match carried {
                    Some(i) => {
                        let mut h = held.remove(i);
                        h.pending.clear();
                        h
                    }
                    None => Member { node: m.node, recognizer: m.recognizer, rejected: false, pending: Vec::new() },
                }
            })
            .collect();
        self.feed(input)
    }

    /// A move, release or cancel of the current press.
    pub fn pointer(&mut self, input: &PointerInput) -> Vec<(NodeId, GestureEvent)> {
        if self.members.is_empty() {
            return Vec::new();
        }
        if matches!(input.phase, PointerPhase::Up | PointerPhase::Cancel) {
            self.pressed = false;
        }
        let out = self.feed(input);
        self.retire();
        out
    }

    /// Runs timers (long press, the double-tap wait) due at `now`.
    pub fn tick(&mut self, now: f64) -> Vec<(NodeId, GestureEvent)> {
        let mut out = Vec::new();
        let mut verdicts = Vec::new();
        for (i, m) in self.members.iter_mut().enumerate() {
            if m.rejected || m.recognizer.deadline().is_none_or(|d| d > now) {
                continue;
            }
            let mut events = Vec::new();
            let verdict = m.recognizer.tick(now, &mut events);
            m.pending.extend(events);
            verdicts.push((i, verdict));
        }
        self.resolve(&verdicts, &mut out);
        self.retire();
        out
    }

    /// The earliest timer of any live member.
    pub fn next_deadline(&self) -> Option<f64> {
        self.members
            .iter()
            .filter(|m| !m.rejected)
            .filter_map(|m| m.recognizer.deadline())
            .min_by(|a, b| a.total_cmp(b))
    }

    /// The node whose recognizer won the current press, if decided.
    pub fn winner(&self) -> Option<NodeId> {
        self.winner.map(|i| self.members[i].node)
    }

    pub fn is_active(&self) -> bool {
        !self.members.is_empty()
    }

    fn feed(&mut self, input: &PointerInput) -> Vec<(NodeId, GestureEvent)> {
        let mut verdicts = Vec::new();
        for (i, m) in self.members.iter_mut().enumerate() {
            if m.rejected {
                continue;
            }
            let mut events = Vec::new();
            let verdict = m.recognizer.input(input, &mut events);
            m.pending.extend(events);
            verdicts.push((i, verdict));
        }
        let mut out = Vec::new();
        self.resolve(&verdicts, &mut out);
        out
    }

    fn resolve(&mut self, verdicts: &[(usize, Verdict)], out: &mut Vec<(NodeId, GestureEvent)>) {
        for &(i, verdict) in verdicts {
            if verdict == Verdict::Reject && self.winner != Some(i) {
                self.members[i].rejected = true;
                self.members[i].pending.clear();
            }
        }
        if self.winner.is_none() {
            // Verdicts are in member order, innermost first: the first acceptance wins a tie.
            let accepted = verdicts.iter().find(|(i, v)| *v == Verdict::Accept && !self.members[*i].rejected);
            if let Some(&(i, _)) = accepted {
                self.win(i, out);
            } else {
                let live: Vec<usize> = (0..self.members.len()).filter(|i| !self.members[*i].rejected).collect();
                if let [only] = live[..] {
                    self.win(only, out);
                }
            }
        }
        if let Some(w) = self.winner {
            let m = &mut self.members[w];
            out.extend(m.pending.drain(..).map(|e| (m.node, e)));
        }
    }

    fn win(&mut self, index: usize, out: &mut Vec<(NodeId, GestureEvent)>) {
        self.winner = Some(index);
        for (i, m) in self.members.iter_mut().enumerate() {
            if i != index && !m.rejected {
                m.rejected = true;
                m.pending.clear();
                let mut cancels = Vec::new();
                m.recognizer.lose(&mut cancels);
                out.extend(cancels.into_iter().map(|e| (m.node, e)));
            }
        }
    }

    /// Ends the session once the pointer is up and no member still waits.
    fn retire(&mut self) {
        if self.pressed {
            return;
        }
        let waiting = self.members.iter().any(|m| !m.rejected && !m.recognizer.finished());
        if !waiting {
            self.members.clear();
            self.winner = None;
        }
    }
}
