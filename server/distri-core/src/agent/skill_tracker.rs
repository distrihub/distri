use std::collections::HashMap;

/// Snapshot of a skill loaded via inline context
#[derive(Debug, Clone)]
pub struct SkillSnapshot {
    pub skill_id: String,
    pub content: String,
    pub loaded_at: i64,
    pub estimated_tokens: usize,
}

/// Tracks skills loaded inline for post-compaction re-injection.
///
/// When compaction drops old scratchpad entries, skill content loaded in earlier
/// turns would be lost. The tracker preserves skill content so it can be
/// re-injected as `SkillContext` scratchpad entries after compaction.
#[derive(Debug, Clone)]
pub struct ActiveSkillTracker {
    skills: HashMap<String, SkillSnapshot>,
    max_reinjection_tokens: usize,
}

impl Default for ActiveSkillTracker {
    fn default() -> Self {
        Self {
            skills: HashMap::new(),
            max_reinjection_tokens: 25_000,
        }
    }
}

impl ActiveSkillTracker {
    pub fn new(max_reinjection_tokens: usize) -> Self {
        Self {
            skills: HashMap::new(),
            max_reinjection_tokens,
        }
    }

    /// Record a skill that was loaded inline. If the same skill_id is loaded
    /// again, the newer version replaces the old one.
    pub fn track(&mut self, skill_id: String, content: String, loaded_at: i64) {
        let estimated_tokens = content.len() / 4; // rough estimate: 4 chars per token
        self.skills.insert(
            skill_id.clone(),
            SkillSnapshot {
                skill_id,
                content,
                loaded_at,
                estimated_tokens,
            },
        );
    }

    /// Return skills to re-inject, ordered most-recently-loaded first,
    /// fitting within the token budget.
    pub fn get_reinjection_candidates(&self) -> Vec<&SkillSnapshot> {
        if self.skills.is_empty() {
            return vec![];
        }

        // Sort by loaded_at descending (most recent first)
        let mut sorted: Vec<&SkillSnapshot> = self.skills.values().collect();
        sorted.sort_by(|a, b| b.loaded_at.cmp(&a.loaded_at));

        let mut result = vec![];
        let mut budget_used = 0;

        for skill in sorted {
            if budget_used + skill.estimated_tokens <= self.max_reinjection_tokens {
                budget_used += skill.estimated_tokens;
                result.push(skill);
            }
        }

        result
    }

    /// Get skill IDs currently tracked
    pub fn tracked_skill_ids(&self) -> Vec<String> {
        self.skills.keys().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_track_and_retrieve() {
        let mut tracker = ActiveSkillTracker::default();
        tracker.track("rubric".into(), "# Rubric\nJSON examples...".into(), 100);

        let candidates = tracker.get_reinjection_candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].skill_id, "rubric");
        assert_eq!(candidates[0].content, "# Rubric\nJSON examples...");
    }

    #[test]
    fn test_most_recent_first() {
        let mut tracker = ActiveSkillTracker::default();
        tracker.track("old_skill".into(), "old content".into(), 100);
        tracker.track("new_skill".into(), "new content".into(), 200);

        let candidates = tracker.get_reinjection_candidates();
        assert_eq!(candidates[0].skill_id, "new_skill");
        assert_eq!(candidates[1].skill_id, "old_skill");
    }

    #[test]
    fn test_token_budget_respected() {
        let mut tracker = ActiveSkillTracker::new(10);
        // 40 chars = 10 tokens (fits alone)
        tracker.track("a".into(), "x".repeat(40), 100);
        // 40 chars = 10 tokens (would push to 20, over budget)
        tracker.track("b".into(), "y".repeat(40), 200);

        let candidates = tracker.get_reinjection_candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].skill_id, "b");
    }

    #[test]
    fn test_replace_on_reload() {
        let mut tracker = ActiveSkillTracker::default();
        tracker.track("rubric".into(), "v1 content".into(), 100);
        tracker.track("rubric".into(), "v2 content".into(), 200);

        let candidates = tracker.get_reinjection_candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].content, "v2 content");
        assert_eq!(candidates[0].loaded_at, 200);
    }

    #[test]
    fn test_empty_tracker() {
        let tracker = ActiveSkillTracker::default();
        assert!(tracker.is_empty());
        assert!(tracker.get_reinjection_candidates().is_empty());
    }

    #[test]
    fn test_clone_is_independent() {
        let mut tracker = ActiveSkillTracker::default();
        tracker.track("skill_a".into(), "content".into(), 100);

        let mut cloned = tracker.clone();
        cloned.track("skill_b".into(), "more content".into(), 200);

        assert_eq!(tracker.tracked_skill_ids().len(), 1);
        assert_eq!(cloned.tracked_skill_ids().len(), 2);
    }
}
