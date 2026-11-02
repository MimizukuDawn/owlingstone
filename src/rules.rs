//! Preference rules (data/rules.yaml): judge each comp for the current game.
//!
//! - `check(cond, ctx)` says if a condition is true, false, or still unknown (None): before the lobby
//!   tribes are sure, a condition on tribes cannot be judged yet.
//! - `judge(comp_id, rules, ctx)` applies every rule of a comp: hidden or not, bonus, notes.
//!
//! Effects (one per rule): only_if (offered only if true), never_if (hidden if true), prefer_if
//! (+bonus when true), avoid_if (-bonus when true). An unknown condition does nothing: a comp is not
//! hidden until we are sure.

use crate::knowledge::{Cond, Effect, Rule};

/// What the rules can look at (None = not known yet).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ctx {
    pub races: Option<Vec<String>>,
    pub deity: Option<String>,
    pub hero: Option<String>,
    /// Names of my current hero powers.
    pub hero_power: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Verdict {
    pub hidden: bool,
    pub bonus: f64,
    pub notes: Vec<String>,
}

/// Some(true) / Some(false), or None while a piece of information is missing.
pub fn check(cond: &Cond, ctx: &Ctx) -> Option<bool> {
    let mut unknown = false;
    if let Some(want) = &cond.hero_power {
        // one of my current hero powers (it can change during the game)
        match &ctx.hero_power {
            None => unknown = true,
            Some(have) => {
                let want: Vec<String> = want.list().iter().map(|w| w.to_lowercase()).collect();
                if !have.iter().any(|h| want.contains(&h.to_lowercase())) {
                    return Some(false);
                }
            }
        }
    }
    for (want, have) in [(&cond.deity, &ctx.deity), (&cond.hero, &ctx.hero)] {
        let Some(want) = want else { continue };
        match have {
            None => unknown = true,
            Some(have) if have.to_lowercase() != want.to_lowercase() => return Some(false),
            Some(_) => {}
        }
    }
    let tribe_conds = [(&cond.lobby_has, 0), (&cond.lobby_has_any, 1), (&cond.lobby_lacks, 2)];
    for (want, kind) in tribe_conds {
        let Some(want) = want else { continue };
        let Some(races) = &ctx.races else {
            unknown = true;
            continue;
        };
        let want = want.list();
        let ok = match kind {
            0 => want.iter().all(|r| races.contains(r)),
            1 => want.iter().any(|r| races.contains(r)),
            _ => !want.iter().any(|r| races.contains(r)),
        };
        if !ok {
            return Some(false);
        }
    }
    if unknown { None } else { Some(true) }
}

/// Result of every rule aimed at this comp.
pub fn judge(comp_id: &str, rules: &[Rule], ctx: &Ctx) -> Verdict {
    let mut v = Verdict::default();
    for rule in rules.iter().filter(|r| r.comps.iter().any(|c| c == comp_id)) {
        let result = check(&rule.cond, ctx);
        match (rule.effect, result) {
            (Effect::OnlyIf, Some(false)) | (Effect::NeverIf, Some(true)) => v.hidden = true,
            (Effect::PreferIf, Some(true)) => v.bonus += rule.bonus,
            (Effect::AvoidIf, Some(true)) => v.bonus -= rule.bonus,
            _ => continue,
        }
        if !rule.note.is_empty() {
            v.notes.push(rule.note.clone());
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::OneOrMany;

    fn races(r: &[&str]) -> Ctx {
        Ctx { races: Some(r.iter().map(|s| s.to_string()).collect()), ..Default::default() }
    }

    fn rule(effect: Effect, cond: Cond) -> Rule {
        Rule { comps: vec!["c".into()], effect, cond, bonus: 3.0, note: "n".into() }
    }

    #[test]
    fn tribe_conditions() {
        let has = Cond { lobby_has: Some(OneOrMany::Many(vec!["PIRATE".into(), "BEAST".into()])), ..Default::default() };
        assert_eq!(check(&has, &races(&["PIRATE", "BEAST"])), Some(true));
        assert_eq!(check(&has, &races(&["PIRATE"])), Some(false));
        assert_eq!(check(&has, &Ctx::default()), None);
        let lacks = Cond { lobby_lacks: Some(OneOrMany::One("BEAST".into())), ..Default::default() };
        assert_eq!(check(&lacks, &races(&["PIRATE"])), Some(true));
        let any = Cond { lobby_has_any: Some(OneOrMany::Many(vec!["DEMON".into(), "DRAGON".into()])), ..Default::default() };
        assert_eq!(check(&any, &races(&["DRAGON"])), Some(true));
    }

    #[test]
    fn deity_hero_and_power_ignore_case() {
        let c = Cond { deity: Some("C'Thun".into()), ..Default::default() };
        let ctx = Ctx { deity: Some("c'thun".into()), ..Default::default() };
        assert_eq!(check(&c, &ctx), Some(true));
        let p = Cond { hero_power: Some(OneOrMany::One("Choose Your Champion".into())), ..Default::default() };
        assert_eq!(check(&p, &Ctx::default()), None);
        let ctx = Ctx { hero_power: Some(vec!["choose your champion".into()]), ..Default::default() };
        assert_eq!(check(&p, &ctx), Some(true));
        // A false condition wins over an unknown one.
        let both = Cond { deity: Some("X".into()), lobby_has: Some(OneOrMany::One("BEAST".into())), ..Default::default() };
        assert_eq!(check(&both, &Ctx { deity: Some("Y".into()), ..Default::default() }), Some(false));
    }

    #[test]
    fn judge_applies_effects() {
        let beast = Cond { lobby_has: Some(OneOrMany::One("BEAST".into())), ..Default::default() };
        let rules = vec![
            rule(Effect::OnlyIf, beast.clone()),
            rule(Effect::PreferIf, Cond::default()),
            rule(Effect::AvoidIf, beast.clone()),
        ];
        let v = judge("c", &rules, &races(&["PIRATE"]));
        assert!(v.hidden);
        assert_eq!(v.bonus, 3.0);
        let v = judge("c", &rules, &races(&["BEAST"]));
        assert!(!v.hidden);
        assert_eq!(v.bonus, 0.0);
        assert_eq!(v.notes.len(), 2);
        // Unknown tribes: nothing hidden.
        assert!(!judge("c", &rules, &Ctx::default()).hidden);
        assert_eq!(judge("other", &rules, &races(&[])), Verdict::default());
    }
}
