//! The Goto panel's matcher: a case-insensitive subsequence match of the
//! query against a candidate, scored so that the rows Fleet puts first come
//! first here too: a match on the file name beats one on its folder, a
//! match at a word start (after `/`, `_`, `-`, `.`, a space, or a lowercase
//! to uppercase step) beats one inside a word, consecutive matched characters
//! beat scattered ones, and a shorter candidate wins a tie. Fleet's exact
//! ranking is not observable; this is the stated assumption until batch 6
//! records enough orderings to refine it (`docs/fleet-probe.md`).

/// A scored match with the candidate's matched char indices, for the
/// highlighted letters in a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i32,
    /// Char indices into the candidate, ascending.
    pub positions: Vec<usize>,
}

const BONUS_WORD_START: i32 = 12;
const BONUS_CAMEL: i32 = 8;
const BONUS_CONSECUTIVE: i32 = 10;
const BONUS_FIRST_CHAR: i32 = 6;
const BONUS_EXACT_PREFIX: i32 = 20;
/// Per skipped candidate char between two matched ones (capped at 10 chars),
/// steep enough that three word-start hits spread over a long name lose to
/// a tight match inside a short one.
const PENALTY_GAP: i32 = 3;
const PENALTY_LEADING_GAP: i32 = 1;

fn is_separator(c: char) -> bool {
    matches!(c, '/' | '\\' | '_' | '-' | '.' | ' ' | ':')
}

/// Matches `query` against `candidate`; `None` when the query's characters
/// do not appear in order. An empty query matches everything with score 0.
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: Vec::new(),
        });
    }
    let cand: Vec<char> = candidate.chars().collect();
    let lower: Vec<char> = cand
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    if q.len() > cand.len() {
        return None;
    }
    // Bonus for landing on each candidate position.
    let bonus: Vec<i32> = (0..cand.len())
        .map(|i| {
            if i == 0 {
                return BONUS_WORD_START + BONUS_FIRST_CHAR;
            }
            let prev = cand[i - 1];
            if is_separator(prev) {
                BONUS_WORD_START
            } else if prev.is_lowercase() && cand[i].is_uppercase() {
                BONUS_CAMEL
            } else if !prev.is_alphanumeric() && cand[i].is_alphanumeric() {
                BONUS_WORD_START / 2
            } else {
                0
            }
        })
        .collect();
    // dp[i][j]: best score matching q[..=i] with q[i] at cand[j]; from[i][j]
    // the j' of q[i-1] that produced it.
    let n = q.len();
    let m = cand.len();
    let none = i32::MIN / 2;
    let mut dp = vec![vec![none; m]; n];
    let mut from = vec![vec![usize::MAX; m]; n];
    for j in 0..m {
        if lower[j] == q[0] {
            dp[0][j] = bonus[j] - PENALTY_LEADING_GAP * (j as i32).min(8);
        }
    }
    for i in 1..n {
        // Best (score, index) over j' < j for the previous row, running.
        let mut best_prev = none;
        let mut best_prev_j = usize::MAX;
        for j in 0..m {
            if j > 0 {
                let pj = j - 1;
                if dp[i - 1][pj] > best_prev {
                    best_prev = dp[i - 1][pj];
                    best_prev_j = pj;
                }
            }
            if lower[j] != q[i] || j == 0 {
                continue;
            }
            // Consecutive with the previous query char, or the best earlier one.
            let mut cands: [(i32, usize); 2] = [(none, usize::MAX), (none, usize::MAX)];
            if dp[i - 1][j - 1] > none {
                cands[0] = (dp[i - 1][j - 1] + BONUS_CONSECUTIVE + bonus[j], j - 1);
            }
            if best_prev > none && best_prev_j != usize::MAX {
                let gap = (j - best_prev_j - 1) as i32;
                cands[1] = (
                    best_prev + bonus[j] - PENALTY_GAP * gap.min(10),
                    best_prev_j,
                );
            }
            let (score, src) = if cands[0].0 >= cands[1].0 {
                cands[0]
            } else {
                cands[1]
            };
            if score > none {
                dp[i][j] = score;
                from[i][j] = src;
            }
        }
    }
    let (end, mut score) = (0..m)
        .filter(|&j| dp[n - 1][j] > none)
        .map(|j| (j, dp[n - 1][j]))
        .max_by_key(|&(j, s)| (s, std::cmp::Reverse(j)))?;
    let mut positions = Vec::with_capacity(n);
    let mut j = end;
    for i in (0..n).rev() {
        positions.push(j);
        if i > 0 {
            j = from[i][j];
        }
    }
    positions.reverse();
    // Whole-candidate bonuses: an exact prefix, and brevity.
    if lower.starts_with(&q) {
        score += BONUS_EXACT_PREFIX;
    }
    if lower.len() == q.len() {
        score += BONUS_EXACT_PREFIX;
    }
    score -= (m as i32 - n as i32).min(40) / 4;
    Some(FuzzyMatch { score, positions })
}

/// Matches a path the way `goto/file` does: against the file name first,
/// then against the whole relative path, taking the better score; a name
/// match is worth more than a path match of the same shape.
pub fn fuzzy_match_path(query: &str, rel_path: &str) -> Option<FuzzyMatch> {
    let name_start = rel_path.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0);
    let name = &rel_path[name_start..];
    let name_chars_before = rel_path[..name_start].chars().count();
    let by_name = fuzzy_match(query, name).map(|mut m| {
        m.score += 30;
        for p in &mut m.positions {
            *p += name_chars_before;
        }
        m
    });
    let by_path = fuzzy_match(query, rel_path);
    match (by_name, by_path) {
        (Some(a), Some(b)) => Some(if a.score >= b.score { a } else { b }),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(q: &str, c: &str) -> i32 {
        fuzzy_match(q, c).map(|m| m.score).unwrap_or(i32::MIN)
    }

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_match("mrs", "main.rs").is_some());
        assert!(fuzzy_match("xyz", "main.rs").is_none());
        assert!(fuzzy_match("", "anything").is_some());
    }

    #[test]
    fn positions_follow_the_candidate() {
        let m = fuzzy_match("mr", "main.rs").unwrap();
        assert_eq!(m.positions, vec![0, 5]);
        let m = fuzzy_match("Main", "src/main.rs").unwrap();
        assert_eq!(m.positions, vec![4, 5, 6, 7]);
    }

    #[test]
    fn ranking() {
        // Word starts beat insides, consecutive beats scattered, short beats long.
        assert!(score("main", "main.rs") > score("main", "domain.rs"));
        assert!(score("fb", "file_bar.rs") > score("fb", "fobar.rs"));
        assert!(score("cfg", "config.json") > score("cfg", "c_long_f_name_g.txt"));
        assert!(score("rec", "Record") > score("rec", "src/direct.rs"));
    }

    #[test]
    fn paths_prefer_the_name() {
        let a = fuzzy_match_path("main", "src/main.rs").unwrap();
        let b = fuzzy_match_path("main", "main/other.rs").unwrap();
        assert!(a.score > b.score);
        assert_eq!(a.positions, vec![4, 5, 6, 7]);
    }
}
