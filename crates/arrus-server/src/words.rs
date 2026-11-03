//! Generated member passwords: `word-word-NN`, easy to read out and type on a phone.

use rand::RngExt;

/// Simple, unambiguous lowercase English words (no homophones, no look-alike letters
/// issues, nothing offensive).
pub const WORDS: &[&str] = &[
    "acorn", "amber", "anchor", "apple", "arrow", "autumn", "badge", "bamboo", "banana", "basket",
    "beach", "berry", "bike", "birch", "blanket", "bloom", "boat", "bottle", "branch", "bread",
    "breeze", "brick", "bridge", "brook", "bubble", "bucket", "butter", "button", "cabin", "cactus",
    "camel", "camera", "candle", "canyon", "carpet", "carrot", "castle", "cedar", "cherry", "circle",
    "cloud", "clover", "coast", "cobalt", "cocoa", "comet", "copper", "coral", "cotton", "cookie",
    "crayon", "cricket", "crystal", "daisy", "desert", "dolphin", "dragon", "dream", "drum", "eagle",
    "earth", "echo", "ember", "falcon", "feather", "fern", "field", "flame", "flute", "forest",
    "fossil", "fox", "garden", "giant", "ginger", "glacier", "globe", "gold", "grape", "gravel",
    "guitar", "harbor", "harp", "hazel", "helmet", "honey", "horizon", "island", "ivory", "jacket",
    "jelly", "jungle", "kettle", "kite", "koala", "ladder", "lagoon", "lantern", "lemon", "lily",
    "lime", "lion", "lotus", "magnet", "mango", "maple", "marble", "meadow", "melon", "meteor",
    "mint", "mirror", "moon", "mountain", "music", "nectar", "noodle", "ocean", "olive", "orange",
    "orbit", "otter", "owl", "paddle", "panda", "paper", "parrot", "peach", "pebble", "pencil",
    "pepper", "piano", "pillow", "pine", "planet", "plum", "pocket", "pony", "poppy", "puzzle",
    "rabbit", "rainbow", "raven", "ribbon", "river", "robin", "rocket", "salmon", "sand", "saturn",
    "shadow", "shell", "silver", "sky", "sloth", "snow", "sparrow", "spark", "spider", "spring",
    "squirrel", "star", "stone", "storm", "sugar", "summer", "sunset", "swan", "table", "tiger",
    "toast", "tomato", "tulip", "tunnel", "turtle", "valley", "velvet", "violet", "volcano", "wagon",
    "walnut", "water", "whale", "willow", "window", "winter", "wolf", "yellow", "zebra", "zigzag",
];

/// A fresh password such as `tiger-maple-42` (two distinct words, a number 10–99).
pub fn generate_password() -> String {
    let mut rng = rand::rng();
    let a = rng.random_range(0..WORDS.len());
    let mut b = rng.random_range(0..WORDS.len() - 1);
    if b >= a {
        b += 1;
    }
    let n: u32 = rng.random_range(10..100);
    format!("{}-{}-{n}", WORDS[a], WORDS[b])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn word_list_is_clean() {
        assert!(WORDS.len() >= 180, "{}", WORDS.len());
        let set: HashSet<_> = WORDS.iter().collect();
        assert_eq!(set.len(), WORDS.len(), "duplicate words");
        for w in WORDS {
            assert!(!w.is_empty() && w.len() <= 8, "{w}");
            assert!(w.chars().all(|c| c.is_ascii_lowercase()), "{w}");
        }
    }

    #[test]
    fn password_format() {
        for _ in 0..500 {
            let p = generate_password();
            let parts: Vec<&str> = p.split('-').collect();
            assert_eq!(parts.len(), 3, "{p}");
            assert!(WORDS.contains(&parts[0]) && WORDS.contains(&parts[1]), "{p}");
            assert_ne!(parts[0], parts[1], "{p}");
            assert_eq!(parts[2].len(), 2, "{p}");
            let n: u32 = parts[2].parse().unwrap();
            assert!((10..100).contains(&n), "{p}");
        }
    }

    #[test]
    fn passwords_vary() {
        let set: HashSet<String> = (0..50).map(|_| generate_password()).collect();
        assert!(set.len() > 40);
    }
}
