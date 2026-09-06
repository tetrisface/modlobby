//! Prints the script a room would play, for reading against BAR's own two
//! lobbies when the format is in question.
//!
//! `cargo run -p skirmish --example script`

fn main() {
    let mut room = skirmish::Room::new(
        "tetrisface",
        "Beyond All Reason test-31134",
        "Supreme Isthmus v2.1",
        "2026.07.04",
    );
    room.set_option("ranked_game", "0");
    room.set_option("experimentallegionfaction", "1");
    room.set_side(3);
    room.add_ai("BARb", "BARb", 1, 1, skirmish::COLOURS[1]);
    room.add_ai("BARb2", "BARb", 2, 1, skirmish::COLOURS[1]);
    room.add_ai("Scavengers", "Scavengers", 3, 2, skirmish::COLOURS[2]);
    room.set_ai_option("BARb", "cheating", "1");
    room.fix_colours();

    let boxes = startbox::Arrangement {
        startboxes: vec![
            corner(0.0, 0.0, 50.0, 200.0),
            corner(75.0, 0.0, 125.0, 200.0),
            corner(150.0, 0.0, 200.0, 200.0),
        ],
    };
    room.set_option(
        "mapmetadata_startbox_override",
        &startbox::encode_override(&boxes).unwrap(),
    );

    print!("{}", room.to_script().script());
}

fn corner(left: f32, top: f32, right: f32, bottom: f32) -> startbox::Box {
    let at = |x: f32, y: f32| startbox::Point {
        x,
        y,
        strength: None,
    };
    startbox::Box {
        poly: vec![at(left, top), at(right, bottom)],
    }
}
