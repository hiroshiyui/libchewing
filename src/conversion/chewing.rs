use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeSet, BinaryHeap},
    fmt::{Debug, Display, Write},
};

use log::{debug, trace};

use super::{Composition, ConversionEngine, Gap, Interval, Outcome, Symbol};
use crate::{
    dictionary::{Dictionary, LookupStrategy, Phrase},
    zhuyin::Syllable,
};

/// The default Chewing conversion method.
#[derive(Debug, Default)]
pub struct ChewingEngine {
    pub(crate) lookup_strategy: LookupStrategy,
}

impl ChewingEngine {
    const MAX_OUT_PATHS: usize = 10;
    /// Creates a new conversion engine.
    pub fn new() -> ChewingEngine {
        ChewingEngine {
            lookup_strategy: LookupStrategy::Standard,
        }
    }
    pub(crate) fn convert<'a>(
        &'a self,
        dict: &'a dyn Dictionary,
        comp: &'a Composition,
    ) -> Vec<Outcome> {
        let paths = {
            if comp.is_empty() {
                return vec![Outcome::default()];
            }
            let (edges, phrases) = self.find_edges(dict, comp);
            if edges.is_empty() {
                return vec![Outcome::default()];
            }
            let paths = self.find_k_paths(Self::MAX_OUT_PATHS, comp.len(), edges, &phrases);
            debug_assert!(!paths.is_empty());
            // TODO: Reranking

            if log::log_enabled!(log::Level::Trace) {
                trace!("Considered paths:");
                trace!("{paths:#?}");
            } else if log::log_enabled!(log::Level::Debug) {
                debug_paths(&paths);
            }
            paths
        };
        paths
            .into_iter()
            .map(|p| {
                let log_prob = p.total_probability();
                let intervals = p
                    .intervals
                    .into_iter()
                    .map(|it| it.into())
                    .fold(vec![], |acc, interval| glue_fn(comp, acc, interval));
                Outcome {
                    intervals,
                    log_prob,
                }
            })
            .collect()
    }
}

impl ConversionEngine for ChewingEngine {
    fn convert<'a>(&'a self, dict: &'a dyn Dictionary, comp: &'a Composition) -> Vec<Outcome> {
        ChewingEngine::convert(self, dict, comp)
    }
}

fn glue_fn(com: &Composition, mut acc: Vec<Interval>, interval: Interval) -> Vec<Interval> {
    if acc.is_empty() {
        acc.push(interval);
        return acc;
    }
    let last = acc.last().expect("acc should have at least one item");
    if !last.is_phrase || !interval.is_phrase {
        acc.push(interval);
        return acc;
    }
    if let Some(Gap::Glue) = com.gap(last.end) {
        let last = acc.pop().expect("acc should have at least one item");
        let mut phrase = last.text.into_string();
        phrase.push_str(&interval.text);
        acc.push(Interval {
            start: last.start,
            end: interval.end,
            is_phrase: true,
            text: phrase.into_boxed_str(),
        })
    } else {
        acc.push(interval);
    }
    acc
}

impl ChewingEngine {
    fn find_best_phrases<D: Dictionary + ?Sized>(
        &self,
        dict: &D,
        start: usize,
        symbols: &[Symbol],
        com: &Composition,
    ) -> Vec<PossiblePhrase> {
        let end = start + symbols.len();

        for i in (start..end).skip(1) {
            if let Some(Gap::Break) = com.gap(i) {
                // There exists a break point that forbids connecting these
                // syllables.
                trace!("No best phrase for {:?} due to break point", symbols);
                return vec![];
            }
        }

        for selection in &com.selections {
            if selection.intersect_range(start, end) && !selection.is_contained_by(start, end) {
                // There's a conflicting partial intersecting selection.
                trace!(
                    "No best phrase for {:?} due to selection {:?}",
                    symbols, selection
                );
                return vec![];
            }
        }

        if symbols.len() == 1
            && let Some(sym) = symbols[0].to_char()
        {
            return vec![PossiblePhrase::Symbol(sym)];
        }

        if symbols.iter().any(|sym| sym.is_char()) {
            return vec![];
        }

        let syllables: Vec<Syllable> = symbols
            .iter()
            .map(|s| s.to_syllable().unwrap_or_default())
            .collect();

        let max_phrases_count = 10;
        let mut phrases = dict
            .lookup(&syllables, self.lookup_strategy)
            .into_iter()
            .filter(|phrase| {
                // If there exists a user selected interval which is a
                // sub-interval of this phrase but the substring is
                // different then we can skip this phrase.
                for selection in &com.selections {
                    debug_assert!(!selection.text.is_empty());
                    if start <= selection.start && end >= selection.end {
                        let offset = selection.start - start;
                        let len = selection.end - selection.start;
                        let substring: String =
                            phrase.as_str().chars().skip(offset).take(len).collect();
                        if substring != selection.text.as_ref() {
                            return false;
                        }
                    }
                }
                true
            })
            .map(|phrase| {
                const MAX: u32 = 9999999;
                debug_assert!((MAX as f64) < TSI_SRC_SUM_FREQ);
                let log_phrase_prob = (phrase.freq().clamp(1, MAX) as f64 / TSI_SRC_SUM_FREQ).ln();
                let log_length_prob: f64 = log_length_prob(syllables.len());
                let log_prob = log_phrase_prob + log_length_prob;
                debug_assert!(log_prob.is_normal());
                PossiblePhrase::Phrase(phrase, log_prob)
            })
            .collect::<Vec<_>>();
        phrases.sort_by(|a, b| f64::total_cmp(&-a.log_prob(), &-b.log_prob()));
        phrases.truncate(max_phrases_count);

        if phrases.is_empty() {
            // try to find if there's a forced selection
            for selection in &com.selections {
                if start == selection.start && end == selection.end {
                    phrases = vec![PossiblePhrase::Phrase(
                        Phrase::new(selection.text.clone(), 0),
                        0.0,
                    )];
                    break;
                }
            }
        }

        trace!("best phraces for {:?} is {:?}", symbols, phrases);
        phrases
    }

    fn find_edges<D: Dictionary + ?Sized>(
        &self,
        dict: &D,
        com: &Composition,
    ) -> (Vec<Edge>, Vec<PossiblePhrase>) {
        let mut sn = 0;
        let mut edges = vec![];
        let mut phrases = vec![];
        for start in 0..com.symbols.len() {
            // No phrase in the dictionary is longer than MAX_PHRASE_LEN syllables.
            const MAX_PHRASE_LEN: usize = 15;
            let max_end = usize::min(start + MAX_PHRASE_LEN, com.symbols.len());
            for end in (start + 1)..=max_end {
                for phrase in self.find_best_phrases(dict, start, &com.symbols[start..end], com) {
                    edges.push(Edge {
                        start,
                        end,
                        sn,
                        cost: -interval_log_prob(&phrase, start, end),
                    });
                    phrases.push(phrase);
                    sn += 1;
                }
            }
        }
        (edges, phrases)
    }

    /// Find K possible best alternative paths.
    ///
    /// This method is modified from [Yen's algorithm][1] to run on a single
    /// source and single sink DAG.
    ///
    /// [1]: https://en.wikipedia.org/wiki/Yen%27s_algorithm
    fn find_k_paths(
        &self,
        k: usize,
        len: usize,
        edges: Vec<Edge>,
        phrases: &[PossiblePhrase],
    ) -> Vec<PossiblePath> {
        let mut graph = vec![vec![]; len];

        for edge in edges.into_iter() {
            graph[edge.start].push(edge);
        }

        n_best_distinct(&graph, len, phrases, k)
    }
}

/// The model score for one phrase occupying [start, end): phrase log-prob
/// plus length prior. Negate for use as a minimization cost.
#[inline]
fn interval_log_prob(phrase: &PossiblePhrase, start: usize, end: usize) -> f64 {
    phrase.log_prob() + log_length_prob(end - start)
}

const TSI_SRC_SUM_FREQ: f64 = 58376702.0;
/// log probability of phrase lenght calculated from tsi.src
#[inline]
fn log_length_prob(len: usize) -> f64 {
    match len {
        1 => -2.063494,
        2 => -0.654789,
        3 => -1.692290,
        4 => -1.872849,
        5 => -4.679164,
        _ => -5.056246,
    }
}

// h[v] = min cost of any path from node v to the sink (len).
// DAG with start < end, so process nodes in decreasing order.
fn future_cost(graph: &[Vec<Edge>], len: usize) -> Vec<f64> {
    let mut h = vec![f64::INFINITY; len + 1];
    h[len] = 0.0;
    for v in (0..len).rev() {
        for e in &graph[v] {
            let c = e.cost + h[e.end];
            if c < h[v] {
                h[v] = c;
            }
        }
    }
    h
}

struct Hyp {
    /// cost so far (source -> node)
    g: f64,
    node: usize,
    /// text produced so far
    surface: String,
    /// index into arena, for path reconstruction
    parent: usize,
    /// the edge taken to get here, for path reconstruction
    edge_sn: usize,
}

/// Modified m-A* algorithm to find the N-best distinct result strings
///
/// Natalia Flerova, Radu Marinescu, and Rina
/// Dechter. 2016. Searching for the M best solutions in graphical
/// models. J. Artif. Int. Res. 55, 1 (January 2016), 889–952.
/// https://jair.org/index.php/jair/article/view/10995
fn n_best_distinct(
    graph: &[Vec<Edge>],
    len: usize,
    phrases: &[PossiblePhrase],
    k: usize,
) -> Vec<PossiblePath> {
    let h = future_cost(graph, len);
    let mut arena: Vec<Hyp> = Vec::new();
    // Min-heap on f = g + h[node]; store Reverse((f_bits, arena_idx)).
    let mut open = BinaryHeap::new();

    arena.push(Hyp {
        g: 0.0,
        node: 0,
        surface: String::new(),
        parent: usize::MAX,
        edge_sn: usize::MAX,
    });
    open.push(Reverse((OrderedF64(h[0]), 0usize)));

    // Recombination: best g already settled for a (node, surface) state.
    let mut closed: BTreeSet<(usize, String)> = BTreeSet::new();
    let mut results = Vec::with_capacity(k);

    while let Some(Reverse((_, idx))) = open.pop() {
        let (node, g) = (arena[idx].node, arena[idx].g);
        let key = (node, arena[idx].surface.clone());
        if !closed.insert(key) {
            // a cheaper path to this exact (node, surface) already won
            continue;
        }

        if node == len {
            // a new distinct reading
            results.push(reconstruct(&arena, idx));
            if results.len() == k {
                break;
            }
            continue;
        }

        for e in &graph[node] {
            let mut surface = arena[idx].surface.clone();
            surface.push_str(&phrases[e.sn].to_string());
            // Prune states already finalized (cheaper)
            if closed.contains(&(e.end, surface.clone())) {
                continue;
            }
            let ng = g + e.cost;
            let child = arena.len();
            arena.push(Hyp {
                g: ng,
                node: e.end,
                surface,
                parent: idx,
                edge_sn: e.sn,
            });
            open.push(Reverse((OrderedF64(ng + h[e.end]), child)));
        }
    }
    results
        .into_iter()
        .map(|edges| {
            let intervals = edges
                .into_iter()
                .map(
                    |Edge {
                         start,
                         end,
                         sn,
                         cost: _,
                     }| PossibleInterval {
                        start,
                        end,
                        phrase: phrases[sn].clone(),
                    },
                )
                .collect();
            PossiblePath { intervals }
        })
        .collect()
}

fn reconstruct(arena: &[Hyp], mut idx: usize) -> Vec<Edge> {
    let mut edges = Vec::new();
    // usize::MAX marks the root sentinel
    while arena[idx].parent != usize::MAX {
        let p = arena[idx].parent;
        edges.push(Edge {
            start: arena[p].node,
            end: arena[idx].node,
            sn: arena[idx].edge_sn,
            cost: arena[idx].g - arena[p].g, // g is cumulative, so the delta is this edge's cost
        });
        idx = p;
    }
    // leaf -> root collected above; flip to source -> sink
    edges.reverse();
    edges
}

#[derive(Clone, Copy, PartialEq)]
struct OrderedF64(f64);

impl Eq for OrderedF64 {}

impl PartialOrd for OrderedF64 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedF64 {
    fn cmp(&self, other: &Self) -> Ordering {
        // total_cmp is a total order over every f64 bit pattern (incl. ±inf, NaN),
        // so Eq/Ord invariants hold and the heap can never panic on comparison.
        self.0.total_cmp(&other.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PossiblePhrase {
    Symbol(char),
    Phrase(Phrase, f64),
}

impl PossiblePhrase {
    fn log_prob(&self) -> f64 {
        match self {
            PossiblePhrase::Symbol(_) => 0.0,
            PossiblePhrase::Phrase(_, log_prob) => *log_prob,
        }
    }
}

impl Display for PossiblePhrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PossiblePhrase::Symbol(sym) => f.write_char(*sym),
            PossiblePhrase::Phrase(phrase, _) => f.write_str(phrase.as_str()),
        }
    }
}

impl From<PossiblePhrase> for Box<str> {
    fn from(value: PossiblePhrase) -> Self {
        match value {
            PossiblePhrase::Symbol(sym) => sym.to_string().into_boxed_str(),
            PossiblePhrase::Phrase(phrase, _) => phrase.into(),
        }
    }
}

#[derive(Debug, Copy, Clone)]
struct Edge {
    start: usize,
    end: usize,
    sn: usize,
    cost: f64,
}

impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        self.sn == other.sn
    }
}

#[derive(Clone, PartialEq)]
struct PossibleInterval {
    start: usize,
    end: usize,
    phrase: PossiblePhrase,
}

impl Debug for PossibleInterval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("I")
            .field(&(self.start..self.end))
            .field(&self.phrase)
            .finish()
    }
}

impl From<PossibleInterval> for Interval {
    fn from(value: PossibleInterval) -> Self {
        Interval {
            start: value.start,
            end: value.end,
            is_phrase: match value.phrase {
                PossiblePhrase::Symbol(_) => false,
                PossiblePhrase::Phrase(_, _) => true,
            },
            text: value.phrase.into(),
        }
    }
}

#[derive(Default, Clone)]
struct PossiblePath {
    intervals: Vec<PossibleInterval>,
}

impl Debug for PossiblePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PossiblePath")
            .field("total_probability()", &self.total_probability())
            .field("intervals", &self.intervals)
            .finish()
    }
}

impl PossiblePath {
    fn total_probability(&self) -> f64 {
        let prob = self.phrase_log_probability();
        debug_assert!(!prob.is_nan());
        prob
    }
    /// Total log-probability of this reading under the calibrated model:
    /// per-interval phrase log-prob PLUS the length prior for that interval.
    /// Must stay in lock-step with the A* `Edge::cost` so the decoder's
    /// emission order matches this score (which derives `Ord`).
    fn phrase_log_probability(&self) -> f64 {
        self.intervals
            .iter()
            .map(|it| interval_log_prob(&it.phrase, it.start, it.end))
            .sum()
    }
}

impl PartialEq for PossiblePath {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl PartialOrd for PossiblePath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PossiblePath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.total_probability()
            .total_cmp(&other.total_probability())
    }
}

impl Eq for PossiblePath {}

impl Display for PossiblePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#P({:.2}", self.total_probability())?;
        for interval in &self.intervals {
            write!(
                f,
                " ({:.2} '{})",
                interval.phrase.log_prob(),
                interval.phrase
            )?;
        }
        write!(f, ")")?;
        Ok(())
    }
}

fn debug_paths(paths: &[PossiblePath]) {
    debug!("Considered paths:");
    for p in paths {
        debug!("  {p}");
    }
}

#[cfg(test)]
mod tests {
    use super::ChewingEngine;
    use crate::{
        conversion::{
            Composition, Gap, Interval, Outcome, Symbol,
            chewing::{Edge, PossibleInterval, PossiblePath, PossiblePhrase, n_best_distinct},
        },
        dictionary::{Dictionary, Phrase, TrieBuf},
        syl,
        zhuyin::Bopomofo::*,
    };

    fn test_dictionary() -> impl Dictionary {
        TrieBuf::from([
            (vec![syl![G, U, O, TONE2]], vec![("國", 1)]),
            (vec![syl![M, I, EN, TONE2]], vec![("民", 1)]),
            (vec![syl![D, A, TONE4]], vec![("大", 1)]),
            (vec![syl![H, U, EI, TONE4]], vec![("會", 1)]),
            (vec![syl![D, AI, TONE4]], vec![("代", 1)]),
            (vec![syl![B, I, AU, TONE3]], vec![("表", 1), ("錶", 1)]),
            (
                vec![syl![G, U, O, TONE2], syl![M, I, EN, TONE2]],
                vec![("國民", 200)],
            ),
            (
                vec![syl![D, A, TONE4], syl![H, U, EI, TONE4]],
                vec![("大會", 200)],
            ),
            (
                vec![syl![D, AI, TONE4], syl![B, I, AU, TONE3]],
                vec![("代表", 200), ("戴錶", 100)],
            ),
            (vec![syl![X, I, EN]], vec![("心", 1)]),
            (vec![syl![K, U, TONE4], syl![I, EN]], vec![("庫音", 300)]),
            (
                vec![syl![X, I, EN], syl![K, U, TONE4], syl![I, EN]],
                vec![("新酷音", 200)],
            ),
            (
                vec![syl![C, E, TONE4], syl![SH, TONE4], syl![I, TONE2]],
                vec![("測試儀", 42)],
            ),
            (
                vec![syl![C, E, TONE4], syl![SH, TONE4]],
                vec![("測試", 9318)],
            ),
            (
                vec![syl![I, TONE2], syl![X, I, A, TONE4]],
                vec![("一下", 10576)],
            ),
            (vec![syl![X, I, A, TONE4]], vec![("下", 10576)]),
            (vec![syl![H, A]], vec![("哈", 1)]),
            (vec![syl![H, A], syl![H, A]], vec![("哈哈", 1)]),
        ])
    }

    #[test]
    fn simple_shortest_path() {
        let graph = vec![
            vec![
                Edge {
                    start: 0,
                    end: 1,
                    sn: 0,
                    cost: 1.0,
                },
                Edge {
                    start: 0,
                    end: 2,
                    sn: 2,
                    cost: 3.0,
                },
            ],
            vec![Edge {
                start: 1,
                end: 2,
                sn: 1,
                cost: 1.0,
            }],
        ];
        let phrases = vec![
            PossiblePhrase::Phrase(Phrase::new("測", 1), 1.0),
            PossiblePhrase::Phrase(Phrase::new("試", 1), 1.0),
            PossiblePhrase::Phrase(Phrase::new("測試", 3), 1.0),
        ];

        assert_eq!(
            vec![PossiblePath {
                intervals: vec![
                    PossibleInterval {
                        start: 0,
                        end: 1,
                        phrase: PossiblePhrase::Phrase(Phrase::new("測", 1), 1.0),
                    },
                    PossibleInterval {
                        start: 1,
                        end: 2,
                        phrase: PossiblePhrase::Phrase(Phrase::new("試", 1), 1.0),
                    }
                ]
            }],
            n_best_distinct(&graph, 2, &phrases, 1)
        );
    }

    #[test]
    fn multi_edge_shortest_path() {
        let graph = vec![
            vec![
                Edge {
                    start: 0,
                    end: 1,
                    sn: 0,
                    cost: 1.0,
                },
                Edge {
                    start: 0,
                    end: 1,
                    sn: 3,
                    cost: 2.0,
                },
                Edge {
                    start: 0,
                    end: 2,
                    sn: 2,
                    cost: 3.0,
                },
            ],
            vec![Edge {
                start: 1,
                end: 2,
                sn: 1,
                cost: 1.0,
            }],
        ];

        let phrases = vec![
            PossiblePhrase::Phrase(Phrase::new("測", 1), 1.0),
            PossiblePhrase::Phrase(Phrase::new("試", 1), 1.0),
            PossiblePhrase::Phrase(Phrase::new("測試", 3), 1.0),
            PossiblePhrase::Phrase(Phrase::new("策", 2), 1.0),
        ];

        assert_eq!(
            vec![PossiblePath {
                intervals: vec![
                    PossibleInterval {
                        start: 0,
                        end: 1,
                        phrase: PossiblePhrase::Phrase(Phrase::new("測", 1), 1.0),
                    },
                    PossibleInterval {
                        start: 1,
                        end: 2,
                        phrase: PossiblePhrase::Phrase(Phrase::new("試", 1), 1.0),
                    }
                ]
            }],
            n_best_distinct(&graph, 2, &phrases, 1)
        );
    }

    #[test]
    fn convert_empty_composition() {
        let dict = test_dictionary();
        let engine = ChewingEngine::new();
        let composition = Composition::new();
        assert_eq!(
            vec![Outcome::default()],
            engine.convert(&dict, &composition)
        );
    }

    // Some corrupted user dictionary may contain empty length syllables
    #[test]
    fn convert_zero_length_entry() {
        let mut dict = test_dictionary();
        dict.add_phrase(&[], ("", 0).into()).unwrap();
        let engine = ChewingEngine::new();
        let mut composition = Composition::new();
        for sym in [
            Symbol::from(syl![C, E, TONE4]),
            Symbol::from(syl![SH, TONE4]),
        ] {
            composition.push(sym);
        }
        assert_eq!(
            vec![Interval {
                start: 0,
                end: 2,
                is_phrase: true,
                text: "測試".into()
            }],
            engine.convert(&dict, &composition)[0].intervals
        );
    }

    #[test]
    fn convert_simple_chinese_composition() {
        let dict = test_dictionary();
        let engine = ChewingEngine::new();
        let mut composition = Composition::new();
        for sym in [
            Symbol::from(syl![G, U, O, TONE2]),
            Symbol::from(syl![M, I, EN, TONE2]),
            Symbol::from(syl![D, A, TONE4]),
            Symbol::from(syl![H, U, EI, TONE4]),
            Symbol::from(syl![D, AI, TONE4]),
            Symbol::from(syl![B, I, AU, TONE3]),
        ] {
            composition.push(sym);
        }
        assert_eq!(
            vec![
                Interval {
                    start: 0,
                    end: 2,
                    is_phrase: true,
                    text: "國民".into()
                },
                Interval {
                    start: 2,
                    end: 4,
                    is_phrase: true,
                    text: "大會".into()
                },
                Interval {
                    start: 4,
                    end: 6,
                    is_phrase: true,
                    text: "代表".into()
                },
            ],
            engine.convert(&dict, &composition)[0].intervals
        );
    }

    #[test]
    fn convert_chinese_composition_with_breaks() {
        let dict = test_dictionary();
        let engine = ChewingEngine::new();
        let mut composition = Composition::new();
        for sym in [
            Symbol::from(syl![G, U, O, TONE2]),
            Symbol::from(syl![M, I, EN, TONE2]),
            Symbol::from(syl![D, A, TONE4]),
            Symbol::from(syl![H, U, EI, TONE4]),
            Symbol::from(syl![D, AI, TONE4]),
            Symbol::from(syl![B, I, AU, TONE3]),
        ] {
            composition.push(sym);
        }
        composition.set_gap(1, Gap::Break);
        composition.set_gap(5, Gap::Break);
        assert_eq!(
            vec![
                Interval {
                    start: 0,
                    end: 1,
                    is_phrase: true,
                    text: "國".into()
                },
                Interval {
                    start: 1,
                    end: 2,
                    is_phrase: true,
                    text: "民".into()
                },
                Interval {
                    start: 2,
                    end: 4,
                    is_phrase: true,
                    text: "大會".into()
                },
                Interval {
                    start: 4,
                    end: 5,
                    is_phrase: true,
                    text: "代".into()
                },
                Interval {
                    start: 5,
                    end: 6,
                    is_phrase: true,
                    text: "表".into()
                },
            ],
            engine.convert(&dict, &composition)[0].intervals
        );
    }

    #[test]
    fn convert_chinese_composition_with_good_selection() {
        let dict = test_dictionary();
        let engine = ChewingEngine::new();
        let mut composition = Composition::new();
        for sym in [
            Symbol::from(syl![G, U, O, TONE2]),
            Symbol::from(syl![M, I, EN, TONE2]),
            Symbol::from(syl![D, A, TONE4]),
            Symbol::from(syl![H, U, EI, TONE4]),
            Symbol::from(syl![D, AI, TONE4]),
            Symbol::from(syl![B, I, AU, TONE3]),
        ] {
            composition.push(sym);
        }
        composition.push_selection(Interval {
            start: 4,
            end: 6,
            is_phrase: true,
            text: "戴錶".into(),
        });
        assert_eq!(
            vec![
                Interval {
                    start: 0,
                    end: 2,
                    is_phrase: true,
                    text: "國民".into()
                },
                Interval {
                    start: 2,
                    end: 4,
                    is_phrase: true,
                    text: "大會".into()
                },
                Interval {
                    start: 4,
                    end: 6,
                    is_phrase: true,
                    text: "戴錶".into()
                },
            ],
            engine.convert(&dict, &composition)[0].intervals
        );
    }

    #[test]
    fn convert_chinese_composition_with_substring_selection() {
        let dict = test_dictionary();
        let engine = ChewingEngine::new();
        let mut composition = Composition::new();
        for sym in [
            Symbol::from(syl![X, I, EN]),
            Symbol::from(syl![K, U, TONE4]),
            Symbol::from(syl![I, EN]),
        ] {
            composition.push(sym);
        }
        composition.push_selection(Interval {
            start: 1,
            end: 3,
            is_phrase: true,
            text: "酷音".into(),
        });
        assert_eq!(
            vec![Interval {
                start: 0,
                end: 3,
                is_phrase: true,
                text: "新酷音".into()
            }],
            engine.convert(&dict, &composition)[0].intervals
        );
    }

    #[test]
    fn multiple_single_word_selection() {
        let dict = test_dictionary();
        let engine = ChewingEngine::new();
        let mut composition = Composition::new();
        for sym in [
            Symbol::from(syl![D, AI, TONE4]),
            Symbol::from(syl![B, I, AU, TONE3]),
        ] {
            composition.push(sym);
        }
        for interval in [
            Interval {
                start: 0,
                end: 1,
                is_phrase: true,
                text: "代".into(),
            },
            Interval {
                start: 1,
                end: 2,
                is_phrase: true,
                text: "錶".into(),
            },
        ] {
            composition.push_selection(interval);
        }
        assert_eq!(
            vec![
                Interval {
                    start: 0,
                    end: 1,
                    is_phrase: true,
                    text: "代".into()
                },
                Interval {
                    start: 1,
                    end: 2,
                    is_phrase: true,
                    text: "錶".into()
                }
            ],
            engine.convert(&dict, &composition)[0].intervals
        );
    }

    #[test]
    fn convert_cycle_alternatives() {
        let dict = test_dictionary();
        let engine = ChewingEngine::new();
        let mut composition = Composition::new();
        for sym in [
            Symbol::from(syl![C, E, TONE4]),
            Symbol::from(syl![SH, TONE4]),
            Symbol::from(syl![I, TONE2]),
            Symbol::from(syl![X, I, A, TONE4]),
        ] {
            composition.push(sym);
        }
        assert_eq!(
            vec![
                Interval {
                    start: 0,
                    end: 2,
                    is_phrase: true,
                    text: "測試".into()
                },
                Interval {
                    start: 2,
                    end: 4,
                    is_phrase: true,
                    text: "一下".into()
                }
            ],
            engine.convert(&dict, &composition)[0].intervals
        );
        assert_eq!(
            vec![
                Interval {
                    start: 0,
                    end: 3,
                    is_phrase: true,
                    text: "測試儀".into()
                },
                Interval {
                    start: 3,
                    end: 4,
                    is_phrase: true,
                    text: "下".into()
                }
            ],
            engine.convert(&dict, &composition)[1].intervals
        );
        assert_eq!(
            Some(vec![
                Interval {
                    start: 0,
                    end: 2,
                    is_phrase: true,
                    text: "測試".into()
                },
                Interval {
                    start: 2,
                    end: 4,
                    is_phrase: true,
                    text: "一下".into()
                }
            ]),
            engine
                .convert(&dict, &composition)
                .into_iter()
                .cycle()
                .nth(2)
                .map(|p| p.intervals)
        );
    }

    #[test]
    fn convert_collapses_equal_text_resegmentations() {
        let dict = test_dictionary();
        let engine = ChewingEngine::new();
        let mut composition = Composition::new();
        for _ in 0..80 {
            composition.push(Symbol::from(syl![H, A]));
        }
        let outcomes = engine.convert(&dict, &composition);
        // Every segmentation of 80 ㄏㄚ (e.g. 40x哈哈, 39x哈哈+2x哈, …) renders to
        // the identical visible string, so de-duplicating by text leaves exactly
        // one candidate instead of dozens of equal-looking re-segmentations.
        assert_eq!(1, outcomes.len());
        // The cheapest segmentation pairs every ㄏㄚ into 哈哈 -> 40 intervals.
        assert_eq!(40, outcomes[0].intervals.len());
    }
}
