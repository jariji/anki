// Copyright: Ankitects Pty Ltd and contributors
// License: GNU AGPL, version 3 or later; http://www.gnu.org/licenses/agpl.html

//! Gathers new cards by taking turns among subdecks. Each round visits every
//! subdeck that still has a card once, in a random order, and takes one card
//! from it. The same rule applies within each subdeck, and a deck's own cards
//! count as one more subdeck. Cards within a deck are taken by ascending
//! position.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

use super::NewCard;
use super::QueueBuilder;
use crate::decks::limits::LimitKind;
use crate::prelude::*;
use crate::storage::card::NewCardSorting;

/// A participant in a deck's round robin.
enum Slot {
    /// The deck's own cards, with the next card last so it can be popped.
    Own(Vec<NewCard>),
    Subdeck(Box<DeckNode>),
}

struct DeckNode {
    deck_id: DeckId,
    slots: Vec<Slot>,
    /// Indices into `slots` not yet visited this round, next one last.
    round: Vec<usize>,
    rng: StdRng,
}

impl DeckNode {
    /// Adds the next card of this deck's subtree to the queue. Returns false
    /// if the subtree is exhausted or its limit has been reached.
    fn draw(&mut self, builder: &mut QueueBuilder) -> Result<bool> {
        if builder.limits.limit_reached(self.deck_id, LimitKind::New)? {
            return Ok(false);
        }
        loop {
            if self.round.is_empty() {
                if self.slots.is_empty() {
                    return Ok(false);
                }
                self.round = (0..self.slots.len()).collect();
                self.round.shuffle(&mut self.rng);
            }
            let idx = *self.round.last().unwrap();
            let drawn = match &mut self.slots[idx] {
                Slot::Own(cards) => draw_own_card(cards, self.deck_id, builder)?,
                Slot::Subdeck(node) => node.draw(builder)?,
            };
            self.round.pop();
            if drawn {
                return Ok(true);
            }
            // slot is exhausted; drop it and fix up the remaining indices
            self.slots.remove(idx);
            for i in &mut self.round {
                if *i > idx {
                    *i -= 1;
                }
            }
        }
    }
}

fn draw_own_card(
    cards: &mut Vec<NewCard>,
    deck_id: DeckId,
    builder: &mut QueueBuilder,
) -> Result<bool> {
    while let Some(card) = cards.pop() {
        if builder.add_new_card(card) {
            builder
                .limits
                .decrement_deck_and_parent_limits(deck_id, LimitKind::New)?;
            return Ok(true);
        }
    }
    Ok(false)
}

impl QueueBuilder {
    pub(super) fn gather_new_cards_by_random_subdecks(
        &mut self,
        col: &mut Collection,
        salt: u32,
    ) -> Result<()> {
        let mut root = self.build_deck_node(col, self.context.root_deck.id, salt)?;
        while !self.limits.root_limit_reached(LimitKind::New) && root.draw(self)? {}
        Ok(())
    }

    fn build_deck_node(&self, col: &Collection, deck_id: DeckId, salt: u32) -> Result<DeckNode> {
        let mut slots = vec![];
        if !self.limits.limit_reached(deck_id, LimitKind::New)? {
            let mut cards = vec![];
            col.storage.for_each_new_card_in_deck(
                deck_id,
                NewCardSorting::LowestPosition,
                |card| {
                    cards.push(card);
                    Ok(true)
                },
            )?;
            if !cards.is_empty() {
                cards.reverse();
                slots.push(Slot::Own(cards));
            }
            for child_id in self.limits.child_deck_ids(deck_id)? {
                let child = self.build_deck_node(col, child_id, salt)?;
                if !child.slots.is_empty() {
                    slots.push(Slot::Subdeck(Box::new(child)));
                }
            }
        }
        // seed per deck so each deck's rounds are stable for the day
        let seed = ((salt as u64) << 32) ^ (deck_id.0 as u64);
        Ok(DeckNode {
            deck_id,
            slots,
            round: vec![],
            rng: StdRng::seed_from_u64(seed),
        })
    }
}
