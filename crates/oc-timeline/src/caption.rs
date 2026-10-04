//! Per-line caption treatment. The model sends a mix. Rust fills what it left out.

use crate::{
    CaptionCue, CaptionEffect, CaptionFont, CaptionMood, CaptionPlace, CaptionRecipe, LineLook,
};

/// A tight shot of a person. Captions stay off the mouth.
/// An empty look is treated as a face so an unlabeled talking head stays low.
#[must_use]
pub fn shot_is_face(look: &str, subject: &str) -> bool {
    let look = look.to_ascii_lowercase();
    let subject = subject.to_ascii_lowercase();
    if subject_is_open(&subject) {
        return false;
    }
    if look_is_open(&look) && !look_is_tight(&look) {
        return false;
    }
    true
}

fn subject_is_open(subject: &str) -> bool {
    ["product", "screen", "object", "landscape", "street"]
        .iter()
        .any(|word| subject.contains(word))
}

fn look_is_open(look: &str) -> bool {
    look.contains("wide")
        || look.contains("action")
        || look.contains("graphic")
        || look.contains("landscape")
        || token(look, &["ews", "ws"])
}

fn look_is_tight(look: &str) -> bool {
    look.contains("close")
        || look.contains("detail")
        || look.contains("medium")
        || token(look, &["ecu", "cu", "mcu", "ms"])
}

fn token(look: &str, words: &[&str]) -> bool {
    look.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| words.contains(&part))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineKind {
    Punch,
    Question,
    Number,
    Explain,
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().filter(|w| !w.is_empty()).count()
}

fn line_kind(text: &str) -> LineKind {
    let trimmed = text.trim();
    let n = word_count(trimmed);
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.ends_with('?')
        || lower.starts_with("why ")
        || lower.starts_with("how ")
        || lower.starts_with("what ")
        || lower.starts_with("who ")
        || lower.starts_with("when ")
        || lower.starts_with("where ")
    {
        return LineKind::Question;
    }
    if trimmed.chars().any(|c| c.is_ascii_digit()) && n > 0 && n <= 8 {
        return LineKind::Number;
    }
    if n > 0 && (n <= 3 || trimmed.ends_with('!')) {
        return LineKind::Punch;
    }
    LineKind::Explain
}

/// Place, font, and effect for one line.
/// The recipe overlays only the fields it sets. `on_face` moves a close person off the mouth.
#[must_use]
pub fn look_for(
    text: &str,
    index: usize,
    recipe: &CaptionRecipe,
    face: bool,
) -> (CaptionPlace, CaptionFont, CaptionEffect) {
    let mood = recipe.base.unwrap_or(CaptionMood::Clean);
    let kind = line_kind(text);
    let n = word_count(text);
    let (mut place, mut font, mut effect) =
        if index == 0 && !face && mood != CaptionMood::Clean && n > 0 && n <= 6 {
            (
                CaptionPlace::Middle,
                CaptionFont::Display,
                CaptionEffect::Typewriter,
            )
        } else {
            fallback(mood, kind, face, n, index)
        };
    let spec = if index == 0 {
        recipe.hook.as_ref().or_else(|| role_look(recipe, kind))
    } else {
        role_look(recipe, kind)
    };
    let explicit_place = spec.and_then(|look| look.place);
    if let Some(look) = spec {
        if let Some(value) = look.place {
            place = value;
        }
        if let Some(value) = look.font {
            font = value;
        }
        if let Some(value) = look.effect {
            effect = value;
        }
    }
    if face {
        if let Some(forced) = recipe.on_face {
            if explicit_place.is_some() || matches!(place, CaptionPlace::Middle | CaptionPlace::Top)
            {
                place = forced;
            }
        } else if explicit_place.is_none() && matches!(place, CaptionPlace::Middle) {
            // The top stays clear of the mouth. Only the middle would cover it.
            place = CaptionPlace::Lower;
        }
    }
    (place, font, effect)
}

fn role_look(recipe: &CaptionRecipe, kind: LineKind) -> Option<&LineLook> {
    match kind {
        LineKind::Punch => recipe.punch.as_ref(),
        LineKind::Question => recipe.question.as_ref(),
        LineKind::Number => recipe.number.as_ref(),
        LineKind::Explain => recipe.explain.as_ref(),
    }
}

fn fallback(
    mood: CaptionMood,
    kind: LineKind,
    face: bool,
    n: usize,
    index: usize,
) -> (CaptionPlace, CaptionFont, CaptionEffect) {
    match (mood, kind, face) {
        (CaptionMood::Clean, LineKind::Punch, _) => (
            CaptionPlace::Bottom,
            CaptionFont::Display,
            CaptionEffect::Pop,
        ),
        (CaptionMood::Clean, LineKind::Question, _) => (
            CaptionPlace::Bottom,
            CaptionFont::Serif,
            CaptionEffect::Fade,
        ),
        (CaptionMood::Clean, LineKind::Number, _) => (
            CaptionPlace::Bottom,
            CaptionFont::Mono,
            CaptionEffect::Typewriter,
        ),
        (CaptionMood::Clean, LineKind::Explain, _) => {
            (CaptionPlace::Bottom, CaptionFont::Sans, CaptionEffect::Fade)
        }
        (CaptionMood::Kinetic | CaptionMood::Bold, LineKind::Question, on_face) => {
            let place = match (on_face, index % 2 == 0) {
                (true, true) => CaptionPlace::Bottom,
                (false, true) => CaptionPlace::Middle,
                _ => CaptionPlace::Top,
            };
            (place, CaptionFont::Serif, CaptionEffect::Fade)
        }
        (CaptionMood::Kinetic, LineKind::Punch, false) if n == 1 && index % 4 == 3 => {
            (CaptionPlace::Top, CaptionFont::Display, CaptionEffect::Pop)
        }
        (CaptionMood::Kinetic, LineKind::Punch, false) => (
            CaptionPlace::Middle,
            CaptionFont::Display,
            if index % 2 == 0 {
                CaptionEffect::Typewriter
            } else {
                CaptionEffect::Pop
            },
        ),
        (CaptionMood::Kinetic, LineKind::Punch, true) => (
            CaptionPlace::Lower,
            CaptionFont::Display,
            CaptionEffect::Pop,
        ),
        (CaptionMood::Kinetic, LineKind::Number, false) => (
            CaptionPlace::Middle,
            CaptionFont::Mono,
            CaptionEffect::Typewriter,
        ),
        (CaptionMood::Kinetic, LineKind::Number, true) => (
            if index % 2 == 0 {
                CaptionPlace::Lower
            } else {
                CaptionPlace::Top
            },
            CaptionFont::Mono,
            CaptionEffect::Typewriter,
        ),
        (CaptionMood::Kinetic, LineKind::Explain, false) if n <= 4 && index % 2 == 1 => {
            (CaptionPlace::Lower, CaptionFont::Sans, CaptionEffect::Fade)
        }
        (CaptionMood::Kinetic, LineKind::Explain, _) => explain_look(index),
        (CaptionMood::Bold, LineKind::Punch, false) => (
            CaptionPlace::Middle,
            CaptionFont::Display,
            CaptionEffect::Pop,
        ),
        (CaptionMood::Bold, LineKind::Punch, true) => (
            CaptionPlace::Lower,
            CaptionFont::Display,
            CaptionEffect::Pop,
        ),
        (CaptionMood::Bold, LineKind::Number, false) => (
            CaptionPlace::Middle,
            CaptionFont::Display,
            CaptionEffect::Typewriter,
        ),
        (CaptionMood::Bold, LineKind::Number, true) => (
            CaptionPlace::Lower,
            CaptionFont::Display,
            CaptionEffect::Typewriter,
        ),
        (CaptionMood::Bold, LineKind::Explain, false) if n <= 5 => (
            CaptionPlace::Lower,
            CaptionFont::Display,
            CaptionEffect::Pop,
        ),
        (CaptionMood::Bold, LineKind::Explain, _) => explain_look(index),
    }
}

/// A long line changes place, face, and effect from one cue to the next.
/// Index 2 stays a low sans vanish, so a face does not lift a line that was already low.
fn explain_look(index: usize) -> (CaptionPlace, CaptionFont, CaptionEffect) {
    match index % 4 {
        0 => (
            CaptionPlace::Top,
            CaptionFont::Display,
            CaptionEffect::Typewriter,
        ),
        1 => (
            CaptionPlace::Lower,
            CaptionFont::Serif,
            CaptionEffect::Fade,
        ),
        2 => (
            CaptionPlace::Bottom,
            CaptionFont::Sans,
            CaptionEffect::Fade,
        ),
        _ => (
            CaptionPlace::Middle,
            CaptionFont::Display,
            CaptionEffect::Pop,
        ),
    }
}

/// Write a look onto each cue. `faces[i]` is true when that line sits on a close person.
/// A missing entry stays off the mouth.
pub fn dress_cues(cues: &mut [CaptionCue], recipe: &CaptionRecipe, faces: &[bool]) {
    for (i, cue) in cues.iter_mut().enumerate() {
        let face = faces.get(i).copied().unwrap_or(true);
        let (place, font, effect) = look_for(&cue.text, i, recipe, face);
        cue.place = place;
        cue.font = font;
        cue.effect = effect;
    }
}

/// Characters revealed so far. Other effects return the whole line.
#[must_use]
pub fn caption_reveal(text: &str, effect: CaptionEffect, into: f64, span: f64) -> String {
    if effect != CaptionEffect::Typewriter {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let t = (into / span.max(0.2)).clamp(0.0, 1.0);
    let p = (t / 0.75).clamp(0.0, 1.0);
    let n = ((p * chars.len() as f64).ceil() as usize).clamp(1, chars.len());
    chars.into_iter().take(n).collect()
}

/// Playhead-locked motion. The monitor rebuilds the node every frame, so this is inline CSS.
#[must_use]
pub fn caption_motion(effect: CaptionEffect, into: f64, span: f64) -> String {
    match effect {
        CaptionEffect::Pop => {
            let p = (into / 0.16).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - p) * (1.0 - p);
            let scale = 0.62 + 0.38 * eased;
            format!("transform:scale({scale:.3});")
        }
        CaptionEffect::Fade => {
            let inn_w = (span * 0.18).clamp(0.06, 0.14);
            let out_w = (span * 0.28).clamp(0.08, 0.36);
            let inn = (into / inn_w).clamp(0.0, 1.0);
            let out = ((span - into) / out_w).clamp(0.0, 1.0);
            format!("opacity:{:.3};", inn * out)
        }
        CaptionEffect::Typewriter | CaptionEffect::None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_close_person_is_a_face_and_a_wide_shot_is_not() {
        assert!(shot_is_face("close", "person"));
        assert!(shot_is_face("close", ""));
        assert!(shot_is_face("", ""));
        assert!(!shot_is_face("bright-wide", ""));
        assert!(!shot_is_face("wide", "person"));
        assert!(!shot_is_face("close", "product"));
        assert!(!shot_is_face("action", ""));
    }

    #[test]
    fn kinetic_punch_on_an_open_shot_lands_in_the_middle() {
        let recipe = CaptionMood::Kinetic.recipe();
        let (place, font, effect) = look_for("Go now", 1, &recipe, false);
        assert_eq!(place, CaptionPlace::Middle);
        assert_eq!(font, CaptionFont::Display);
        assert!(matches!(
            effect,
            CaptionEffect::Pop | CaptionEffect::Typewriter
        ));
    }

    #[test]
    fn kinetic_punch_on_a_face_stays_under_it() {
        let recipe = CaptionMood::Kinetic.recipe();
        let (place, font, _) = look_for("Go now", 1, &recipe, true);
        assert_eq!(place, CaptionPlace::Lower);
        assert_eq!(font, CaptionFont::Display);
        assert_ne!(place, CaptionPlace::Middle);
    }

    #[test]
    fn a_question_fades_and_a_number_types_on() {
        let clean = CaptionMood::Clean.recipe();
        let (_, _, question) = look_for("why this street?", 2, &clean, true);
        assert_eq!(question, CaptionEffect::Fade);
        let kinetic = CaptionMood::Kinetic.recipe();
        let (place, font, effect) = look_for("3 steps", 2, &kinetic, false);
        assert_eq!(place, CaptionPlace::Middle);
        assert_eq!(font, CaptionFont::Mono);
        assert_eq!(effect, CaptionEffect::Typewriter);
    }

    #[test]
    fn a_clean_explanation_stays_low_and_vanishes() {
        let recipe = CaptionMood::Clean.recipe();
        let (place, font, effect) = look_for("the city opens up", 0, &recipe, false);
        assert_eq!(place, CaptionPlace::Bottom);
        assert_eq!(font, CaptionFont::Sans);
        assert_eq!(effect, CaptionEffect::Fade);
    }

    #[test]
    fn the_hook_on_an_open_kinetic_shot_types_on_in_the_middle() {
        let recipe = CaptionMood::Kinetic.recipe();
        let (place, font, effect) = look_for("wait for this", 0, &recipe, false);
        assert_eq!(place, CaptionPlace::Middle);
        assert_eq!(font, CaptionFont::Display);
        assert_eq!(effect, CaptionEffect::Typewriter);
    }

    #[test]
    fn typewriter_reveals_and_then_holds() {
        assert_eq!(
            caption_reveal("Go", CaptionEffect::Typewriter, 0.0, 1.0)
                .chars()
                .count(),
            1
        );
        assert_eq!(
            caption_reveal("Go", CaptionEffect::Typewriter, 1.0, 1.0),
            "Go"
        );
        assert_eq!(caption_reveal("Go", CaptionEffect::Pop, 0.0, 1.0), "Go");
    }

    #[test]
    fn fade_is_transparent_at_the_tail() {
        let css = caption_motion(CaptionEffect::Fade, 1.9, 2.0);
        assert!(css.starts_with("opacity:0"), "{css}");
        let pop = caption_motion(CaptionEffect::Pop, 0.0, 2.0);
        assert!(pop.contains("scale(0.62"), "{pop}");
    }

    #[test]
    fn a_recipe_puts_a_punch_anywhere_and_a_face_keeps_the_rest() {
        let mut recipe = CaptionRecipe::default();
        recipe.punch = Some(LineLook::parse("top serif fade"));
        recipe.on_face = Some(CaptionPlace::Lower);
        let (place, font, effect) = look_for("Go now", 1, &recipe, false);
        assert_eq!(place, CaptionPlace::Top);
        assert_eq!(font, CaptionFont::Serif);
        assert_eq!(effect, CaptionEffect::Fade);
        let (place, font, effect) = look_for("Go now", 1, &recipe, true);
        assert_eq!(place, CaptionPlace::Lower);
        assert_eq!(font, CaptionFont::Serif);
        assert_eq!(effect, CaptionEffect::Fade);
    }

    #[test]
    fn an_explicit_middle_stays_on_a_face_when_on_face_is_omitted() {
        let mut recipe = CaptionRecipe {
            base: Some(CaptionMood::Clean),
            ..CaptionRecipe::default()
        };
        recipe.punch = Some(LineLook::parse("middle display typewriter"));
        let (place, font, effect) = look_for("Go now", 1, &recipe, true);
        assert_eq!(place, CaptionPlace::Middle);
        assert_eq!(font, CaptionFont::Display);
        assert_eq!(effect, CaptionEffect::Typewriter);
    }

    #[test]
    fn on_face_does_not_lift_a_line_that_was_already_low() {
        let recipe = CaptionMood::Kinetic.recipe();
        let (place, _, effect) = look_for("the city opens up from here", 2, &recipe, true);
        assert_eq!(place, CaptionPlace::Bottom);
        assert_eq!(effect, CaptionEffect::Fade);
    }

    #[test]
    fn a_hook_overrides_the_opening_line() {
        let mut recipe = CaptionMood::Kinetic.recipe();
        recipe.hook = Some(LineLook::parse("bottom mono none"));
        let (place, font, effect) = look_for("wait for this", 0, &recipe, false);
        assert_eq!(place, CaptionPlace::Bottom);
        assert_eq!(font, CaptionFont::Mono);
        assert_eq!(effect, CaptionEffect::None);
    }

    #[test]
    fn a_phrase_keeps_only_the_words_it_knows() {
        let raw = r#"{"punch":"top serif fade","explain":"bottom sans none","on_face":"middle"}"#;
        let recipe: CaptionRecipe = serde_json::from_str(raw).unwrap();
        let punch = recipe.punch.expect("punch");
        assert_eq!(punch.place, Some(CaptionPlace::Top));
        assert_eq!(punch.font, Some(CaptionFont::Serif));
        assert_eq!(punch.effect, Some(CaptionEffect::Fade));
        assert_eq!(
            recipe.explain.expect("explain").effect,
            Some(CaptionEffect::None)
        );
        assert_eq!(recipe.on_face, Some(CaptionPlace::Middle));
        let loose = LineLook::parse("sparkle top wobble serif");
        assert_eq!(loose.place, Some(CaptionPlace::Top));
        assert_eq!(loose.font, Some(CaptionFont::Serif));
        assert!(loose.effect.is_none());
    }

    #[test]
    fn a_loose_note_overrides_one_role_and_prose_overrides_nothing() {
        let one = CaptionRecipe::from_loose("hook top display typewriter").expect("hook");
        let hook = one.hook.expect("hook");
        assert_eq!(hook.place, Some(CaptionPlace::Top));
        assert_eq!(hook.font, Some(CaptionFont::Display));
        assert_eq!(hook.effect, Some(CaptionEffect::Typewriter));
        assert!(one.punch.is_none());
        assert_eq!(one.on_face, Some(CaptionPlace::Lower));
        let bias = CaptionRecipe::from_loose("serif fade").expect("bias");
        let punch = bias.punch.expect("punch");
        assert!(punch.place.is_none());
        assert_eq!(punch.font, Some(CaptionFont::Serif));
        assert_eq!(punch.effect, Some(CaptionEffect::Fade));
        assert!(bias.on_face.is_none());
        assert!(CaptionRecipe::from_loose("make it funky").is_none());
    }

    #[test]
    fn a_kinetic_explanation_changes_place_and_a_face_can_take_the_top() {
        let recipe = CaptionMood::Kinetic.recipe();
        assert!(recipe.on_face.is_none());
        let line = "the city opens up from here";
        let (top, top_font, top_effect) = look_for(line, 0, &recipe, true);
        assert_eq!(top, CaptionPlace::Top);
        assert_eq!(top_font, CaptionFont::Display);
        assert_eq!(top_effect, CaptionEffect::Typewriter);
        let (lower, lower_font, lower_effect) = look_for(line, 1, &recipe, true);
        assert_eq!(lower, CaptionPlace::Lower);
        assert_eq!(lower_font, CaptionFont::Serif);
        assert_eq!(lower_effect, CaptionEffect::Fade);
        let (low, _, low_effect) = look_for(line, 2, &recipe, true);
        assert_eq!(low, CaptionPlace::Bottom);
        assert_eq!(low_effect, CaptionEffect::Fade);
        let (under, under_font, under_effect) = look_for(line, 3, &recipe, true);
        assert_eq!(under, CaptionPlace::Lower);
        assert_eq!(under_font, CaptionFont::Display);
        assert_eq!(under_effect, CaptionEffect::Pop);
        assert_ne!(top, lower);
        assert_ne!(lower_font, top_font);
    }
}
