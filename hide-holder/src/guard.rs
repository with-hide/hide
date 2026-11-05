//! What a guarded submission checks of the screen
//! (`hide_holder_protocol::Guard`, with-hide/hide-private#545): whether the
//! text it typed showed, judged on the text itself and never on any redraw,
//! since a starting program that animates redraws while it drops keys.

/// How often `needles` show on `screen`, each line compared with every
/// space left out: a needle is the typed text's own end or an agent's paste
/// chip, and an agent draws either inside its composer's borders, where a
/// wrapped line or a padded box puts spaces the text did not have.
pub fn shown(screen: &str, needles: &[String]) -> usize {
    let needles: Vec<String> = needles.iter().map(|needle| squeezed(needle)).collect();
    screen
        .lines()
        .map(|line| {
            let line = squeezed(line);
            needles
                .iter()
                .filter(|needle| !needle.is_empty())
                .map(|needle| line.matches(needle.as_str()).count())
                .sum::<usize>()
        })
        .sum()
}

fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn needles(needles: &[&str]) -> Vec<String> {
        needles.iter().map(|needle| (*needle).to_owned()).collect()
    }

    #[test]
    fn typed_text_or_a_paste_chip_shows_and_a_redraw_does_not() {
        let tail = needles(&["JOURNEY-3EC1681D", "[Pasted ~"]);
        let empty = "  ┃\n  ┃  Build auto · Journey\n  ╹▀▀▀▀\n";
        assert_eq!(shown(empty, &tail), 0);
        let spinner = "  ⠋ Loading\n  ┃\n  ┃  Build auto · Journey\n";
        assert_eq!(shown(spinner, &tail), 0, "a redraw is no landing");
        let typed = "  ┃  Reply with exactly JOURNEY-3EC1681D\n  ┃  Build auto\n";
        assert_eq!(shown(typed, &tail), 1);
        let chip = "  ┃  [Pasted ~18 lines]\n";
        assert_eq!(shown(chip, &tail), 1);
        let padded = "│ > [Pasted:  12 lines] │";
        assert_eq!(shown(padded, &needles(&["[Pasted: "])), 1);
    }

    #[test]
    fn a_needle_already_on_the_screen_counts_once_for_each_place() {
        let tail = needles(&["JOURNEY-3EC1681D"]);
        let before = "you: JOURNEY-3EC1681D\n› \n";
        let after = "you: JOURNEY-3EC1681D\n› JOURNEY-3EC1681D\n";
        assert!(shown(after, &tail) > shown(before, &tail));
    }
}
