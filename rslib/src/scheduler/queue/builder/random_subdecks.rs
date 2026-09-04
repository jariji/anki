// Copyright: Ankitects Pty Ltd and contributors
// License: GNU AGPL, version 3 or later; http://www.gnu.org/licenses/agpl.html

//! Gathers new cards by taking turns among subdecks. Each round visits every
//! subdeck that still has a card once, in a random order, and takes one card
//! from it. A deck's own cards count as one more subdeck.
//!
//! Each subdeck supplies its cards according to its own gather order:
//! - `Random subdecks`: the same round robin, recursively.
//! - `Deck` and `Deck, then random notes`: its own cards, then each of its
//!   subdecks in name order, each recursively.
//! - The position and random orders: its whole subtree as one sorted list.
//!
//! Cards within a deck are taken by ascending position unless the deck's
//! order says otherwise.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

use super::NewCard;
use super::QueueBuilder;
use crate::deckconfig::NewCardGatherPriority;
use crate::decks::limits::LimitKind;
use crate::prelude::*;
use crate::storage::card::NewCardSorting;

/// A participant in a deck's gathering.
enum Slot {
    /// Cards in gather order, with the next card last so it can be popped.
    Cards(Vec<NewCard>),
    Subdeck(Box<DeckNode>),
}

enum Policy {
    /// Each round visits every slot once in a fresh random order.
    RoundRobin,
    /// Slots are drained one after another in order.
    Sequential,
}

struct DeckNode {
    deck_id: DeckId,
    policy: Policy,
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
            if self.slots.is_empty() {
                return Ok(false);
            }
            let idx = match self.policy {
                Policy::Sequential => 0,
                Policy::RoundRobin => {
                    if self.round.is_empty() {
                        self.round = (0..self.slots.len()).collect();
                        self.round.shuffle(&mut self.rng);
                    }
                    *self.round.last().unwrap()
                }
            };
            let drawn = match &mut self.slots[idx] {
                Slot::Cards(cards) => draw_card(cards, builder)?,
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

/// Adds the next card whose deck limit has not been reached, if any.
fn draw_card(cards: &mut Vec<NewCard>, builder: &mut QueueBuilder) -> Result<bool> {
    while let Some(card) = cards.pop() {
        let deck_id = card.current_deck_id;
        if builder.limits.limit_reached(deck_id, LimitKind::New)? {
            continue;
        }
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
        let mut policy = Policy::Sequential;
        if !self.limits.limit_reached(deck_id, LimitKind::New)? {
            match self.gather_priority_of(deck_id) {
                NewCardGatherPriority::RandomSubdecks => {
                    policy = Policy::RoundRobin;
                    self.add_deck_and_subdeck_slots(
                        col,
                        deck_id,
                        NewCardSorting::LowestPosition,
                        salt,
                        &mut slots,
                    )?;
                }
                NewCardGatherPriority::Deck => {
                    self.add_deck_and_subdeck_slots(
                        col,
                        deck_id,
                        NewCardSorting::LowestPosition,
                        salt,
                        &mut slots,
                    )?;
                }
                NewCardGatherPriority::DeckThenRandomNotes => {
                    self.add_deck_and_subdeck_slots(
                        col,
                        deck_id,
                        NewCardSorting::RandomNotes(salt),
                        salt,
                        &mut slots,
                    )?;
                }
                NewCardGatherPriority::LowestPosition => {
                    self.add_subtree_slot(col, deck_id, NewCardSorting::LowestPosition, &mut slots)?
                }
                NewCardGatherPriority::HighestPosition => self.add_subtree_slot(
                    col,
                    deck_id,
                    NewCardSorting::HighestPosition,
                    &mut slots,
                )?,
                NewCardGatherPriority::RandomNotes => self.add_subtree_slot(
                    col,
                    deck_id,
                    NewCardSorting::RandomNotes(salt),
                    &mut slots,
                )?,
                NewCardGatherPriority::RandomCards => self.add_subtree_slot(
                    col,
                    deck_id,
                    NewCardSorting::RandomCards(salt),
                    &mut slots,
                )?,
            }
        }
        // seed per deck so each deck's rounds are stable for the day
        let seed = ((salt as u64) << 32) ^ (deck_id.0 as u64);
        Ok(DeckNode {
            deck_id,
            policy,
            slots,
            round: vec![],
            rng: StdRng::seed_from_u64(seed),
        })
    }

    /// The deck's own cards, then a node for each subdeck.
    fn add_deck_and_subdeck_slots(
        &self,
        col: &Collection,
        deck_id: DeckId,
        own_sort: NewCardSorting,
        salt: u32,
        slots: &mut Vec<Slot>,
    ) -> Result<()> {
        let mut cards = vec![];
        col.storage
            .for_each_new_card_in_deck(deck_id, own_sort, |card| {
                cards.push(card);
                Ok(true)
            })?;
        if !cards.is_empty() {
            cards.reverse();
            slots.push(Slot::Cards(cards));
        }
        for child_id in self.limits.child_deck_ids(deck_id)? {
            let child = self.build_deck_node(col, child_id, salt)?;
            if !child.slots.is_empty() {
                slots.push(Slot::Subdeck(Box::new(child)));
            }
        }
        Ok(())
    }

    /// All cards of the deck and its descendants as one sorted list.
    fn add_subtree_slot(
        &self,
        col: &Collection,
        deck_id: DeckId,
        sort: NewCardSorting,
        slots: &mut Vec<Slot>,
    ) -> Result<()> {
        let mut decks = vec![];
        self.collect_subtree_deck_ids(deck_id, &mut decks)?;
        let mut cards = vec![];
        col.storage
            .for_each_new_card_in_decks(&decks, sort, |card| {
                cards.push(card);
                Ok(true)
            })?;
        if !cards.is_empty() {
            cards.reverse();
            slots.push(Slot::Cards(cards));
        }
        Ok(())
    }

    fn collect_subtree_deck_ids(&self, deck_id: DeckId, out: &mut Vec<DeckId>) -> Result<()> {
        out.push(deck_id);
        for child_id in self.limits.child_deck_ids(deck_id)? {
            self.collect_subtree_deck_ids(child_id, out)?;
        }
        Ok(())
    }

    /// The deck's configured gather order, or the default for filtered decks.
    fn gather_priority_of(&self, deck_id: DeckId) -> NewCardGatherPriority {
        self.context
            .deck_map
            .get(&deck_id)
            .and_then(|deck| deck.config_id())
            .and_then(|config_id| self.context.config_map.get(&config_id))
            .map(|config| config.inner.new_card_gather_priority())
            .unwrap_or_default()
    }
}
