//! One caption theme for the whole video. The model picks it. Rust only moves a line
//! so it stays off a close mouth and inside the frame.

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

/// Place, font, and effect for one line.
/// Font and effect come from the video's one theme. Place moves only to stay off a mouth.
#[must_use]
pub fn look_for(
    text: &str,
    index: usize,
    recipe: &CaptionRecipe,
    face: bool,
) -> (CaptionPlace, CaptionFont, CaptionEffect) {
    let _ = (text, index);
    let (place, font, effect) = theme_of(recipe);
    (seat(place, face, recipe.on_face), font, effect)
}

fn theme_of(recipe: &CaptionRecipe) -> (CaptionPlace, CaptionFont, CaptionEffect) {
    let mood = recipe.base.unwrap_or(CaptionMood::Clean);
    let (mut place, mut font, mut effect) = mood_theme(mood);
    if let Some(look) = explicit_look(recipe) {
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
    (place, font, effect)
}

/// The first look the model set is the theme for every line.
fn explicit_look(recipe: &CaptionRecipe) -> Option<&LineLook> {
    recipe
        .hook
        .as_ref()
        .or(recipe.punch.as_ref())
        .or(recipe.explain.as_ref())
        .or(recipe.question.as_ref())
        .or(recipe.number.as_ref())
}

fn mood_theme(mood: CaptionMood) -> (CaptionPlace, CaptionFont, CaptionEffect) {
    match mood {
        CaptionMood::Clean => (CaptionPlace::Bottom, CaptionFont::Sans, CaptionEffect::Fade),
        CaptionMood::Kinetic => (
            CaptionPlace::Lower,
            CaptionFont::Display,
            CaptionEffect::Pop,
        ),
        CaptionMood::Bold => (
            CaptionPlace::Bottom,
            CaptionFont::Display,
            CaptionEffect::Pop,
        ),
    }
}

/// A close face does not take the middle. That band covers the mouth.
fn seat(place: CaptionPlace, face: bool, on_face: Option<CaptionPlace>) -> CaptionPlace {
    if !face {
        return place;
    }
    let chosen = on_face.unwrap_or(place);
    if chosen == CaptionPlace::Middle {
        CaptionPlace::Lower
    } else {
        chosen
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
    fn one_theme_is_the_same_on_every_line() {
        let kinetic = CaptionMood::Kinetic.recipe();
        let lines = ["Go now", "why this street?", "3 steps", "the city opens up"];
        let first = look_for(lines[0], 0, &kinetic, false);
        for (index, line) in lines.iter().enumerate() {
            assert_eq!(look_for(line, index, &kinetic, false), first);
        }
        assert_eq!(first.0, CaptionPlace::Lower);
        assert_eq!(first.1, CaptionFont::Display);
        assert_eq!(first.2, CaptionEffect::Pop);
        let clean = CaptionMood::Clean.recipe();
        let (place, font, effect) = look_for("why this street?", 2, &clean, false);
        assert_eq!(
            (place, font, effect),
            look_for("the city opens up", 0, &clean, true)
        );
        assert_eq!(place, CaptionPlace::Bottom);
        assert_eq!(font, CaptionFont::Sans);
        assert_eq!(effect, CaptionEffect::Fade);
    }

    #[test]
    fn a_face_keeps_the_theme_and_moves_off_the_mouth() {
        let mut recipe = CaptionMood::Bold.recipe();
        recipe.punch = Some(LineLook::parse("middle display pop"));
        let open = look_for("Go now", 1, &recipe, false);
        let face = look_for("the city opens up from here", 4, &recipe, true);
        assert_eq!(open.0, CaptionPlace::Middle);
        assert_eq!(face.0, CaptionPlace::Lower);
        assert_eq!(open.1, face.1);
        assert_eq!(open.2, face.2);
        assert_eq!(face.1, CaptionFont::Display);
        assert_eq!(face.2, CaptionEffect::Pop);
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
    fn a_middle_theme_on_a_face_drops_under_the_mouth() {
        let mut recipe = CaptionRecipe {
            base: Some(CaptionMood::Clean),
            ..CaptionRecipe::default()
        };
        recipe.punch = Some(LineLook::parse("middle display typewriter"));
        let (place, font, effect) = look_for("Go now", 1, &recipe, true);
        assert_eq!(place, CaptionPlace::Lower);
        assert_eq!(font, CaptionFont::Display);
        assert_eq!(effect, CaptionEffect::Typewriter);
        let (open, _, _) = look_for("the long explanation stays", 3, &recipe, false);
        assert_eq!(open, CaptionPlace::Middle);
        assert_eq!(
            look_for("why now?", 2, &recipe, false).1,
            CaptionFont::Display
        );
    }

    #[test]
    fn a_low_theme_stays_low_on_a_face() {
        let recipe = CaptionMood::Kinetic.recipe();
        let (place, font, effect) = look_for("the city opens up from here", 2, &recipe, true);
        assert_eq!(place, CaptionPlace::Lower);
        assert_eq!(font, CaptionFont::Display);
        assert_eq!(effect, CaptionEffect::Pop);
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
    fn a_kinetic_explanation_keeps_one_theme() {
        let recipe = CaptionMood::Kinetic.recipe();
        assert!(recipe.on_face.is_none());
        let line = "the city opens up from here";
        let first = look_for(line, 0, &recipe, true);
        for index in 1..4 {
            assert_eq!(look_for(line, index, &recipe, true), first);
        }
        assert_eq!(first.0, CaptionPlace::Lower);
        assert_eq!(first.1, CaptionFont::Display);
        assert_eq!(first.2, CaptionEffect::Pop);
        assert_ne!(first.0, CaptionPlace::Middle);
        assert_ne!(first.0, CaptionPlace::Top);
    }
}
