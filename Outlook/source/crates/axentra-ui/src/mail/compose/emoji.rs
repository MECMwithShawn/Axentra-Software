//! Emoji for the Insert > Emoji picker: (emoji, search name), by category.

/// (emoji, search name).
pub type Emoji = (&'static str, &'static str);

/// (id, label, emoji) for each category tab.
pub const CATEGORIES: &[(&str, &str, &[Emoji])] = &[
    ("smileys", "Smileys & emotion", &[
        ("😀", "grinning face"), ("😃", "grinning face with big eyes"), ("😄", "grinning face with smiling eyes"),
        ("😁", "beaming face"), ("😆", "laughing"), ("😅", "sweat smile"), ("🤣", "rolling on the floor laughing"),
        ("😂", "tears of joy"), ("🙂", "slightly smiling face"), ("🙃", "upside-down face"), ("😉", "winking face"),
        ("😊", "smiling face with smiling eyes"), ("😇", "halo innocent"), ("🥰", "smiling face with hearts"),
        ("😍", "heart eyes"), ("🤩", "star-struck"), ("😘", "blowing a kiss"), ("😋", "yum savoring food"),
        ("😜", "winking face with tongue"), ("🤪", "zany face"), ("🤗", "hugging face"), ("🤭", "hand over mouth"),
        ("🤫", "shushing face"), ("🤔", "thinking face"), ("🤐", "zipper mouth"), ("😐", "neutral face"),
        ("😑", "expressionless"), ("😶", "no mouth"), ("😏", "smirking face"), ("😒", "unamused"),
        ("🙄", "rolling eyes"), ("😬", "grimacing"), ("😌", "relieved"), ("😔", "pensive"), ("😪", "sleepy"),
        ("😴", "sleeping"), ("😷", "face with medical mask"), ("🤒", "thermometer sick"), ("🤯", "exploding head mind blown"),
        ("🥳", "partying face celebrate"), ("😎", "sunglasses cool"), ("🤓", "nerd face"), ("🧐", "monocle"),
        ("😕", "confused"), ("😟", "worried"), ("🙁", "slightly frowning"), ("😮", "open mouth surprised"),
        ("😲", "astonished"), ("😳", "flushed"), ("🥺", "pleading face"), ("😢", "crying face"), ("😭", "loudly crying"),
        ("😱", "screaming in fear"), ("😖", "confounded"), ("😞", "disappointed"), ("😓", "downcast sweat"),
        ("😩", "weary"), ("😫", "tired face"), ("🥱", "yawning"), ("😤", "triumph huff"), ("😡", "pouting angry"),
        ("😠", "angry face"), ("🤬", "cursing"), ("💩", "pile of poo"), ("🤡", "clown"), ("👻", "ghost"),
        ("🤖", "robot"), ("❤️", "red heart love"), ("🧡", "orange heart"), ("💛", "yellow heart"), ("💚", "green heart"),
        ("💙", "blue heart"), ("💜", "purple heart"), ("🖤", "black heart"), ("💔", "broken heart"), ("💯", "hundred points"),
        ("💥", "collision boom"), ("💫", "dizzy"), ("💬", "speech balloon"), ("💤", "zzz"),
    ]),
    ("people", "People & body", &[
        ("👋", "waving hand hello"), ("🤚", "raised back of hand"), ("✋", "raised hand"), ("🖖", "vulcan salute"),
        ("👌", "ok hand"), ("🤌", "pinched fingers"), ("✌️", "victory hand"), ("🤞", "crossed fingers luck"),
        ("🤟", "love-you gesture"), ("🤘", "sign of the horns"), ("🤙", "call me hand"), ("👈", "pointing left"),
        ("👉", "pointing right"), ("👆", "pointing up"), ("👇", "pointing down"), ("☝️", "index pointing up"),
        ("👍", "thumbs up like yes"), ("👎", "thumbs down no"), ("✊", "raised fist"), ("👊", "oncoming fist"),
        ("👏", "clapping hands applause"), ("🙌", "raising hands celebration"), ("👐", "open hands"), ("🤲", "palms up"),
        ("🤝", "handshake deal"), ("🙏", "folded hands please thanks"), ("✍️", "writing hand"), ("💪", "flexed biceps strong"),
        ("🧠", "brain"), ("👀", "eyes look"), ("👶", "baby"), ("🧑", "person"), ("👩", "woman"), ("👨", "man"),
        ("🧑‍💻", "technologist developer"), ("🧑‍🏫", "teacher"), ("🧑‍🍳", "cook"), ("🧑‍🚀", "astronaut"),
        ("🙋", "person raising hand"), ("🤷", "person shrugging"), ("🤦", "person facepalming"), ("🏃", "person running"),
        ("💃", "woman dancing"), ("👪", "family"),
    ]),
    ("animals", "Animals & nature", &[
        ("🐶", "dog face"), ("🐱", "cat face"), ("🐭", "mouse face"), ("🐰", "rabbit face"), ("🦊", "fox"),
        ("🐻", "bear"), ("🐼", "panda"), ("🐨", "koala"), ("🐯", "tiger face"), ("🦁", "lion"), ("🐮", "cow face"),
        ("🐷", "pig face"), ("🐸", "frog"), ("🐵", "monkey face"), ("🐔", "chicken"), ("🐧", "penguin"),
        ("🐦", "bird"), ("🦉", "owl"), ("🐝", "honeybee"), ("🦋", "butterfly"), ("🐢", "turtle"), ("🐙", "octopus"),
        ("🐬", "dolphin"), ("🐳", "spouting whale"), ("🌸", "cherry blossom"), ("🌹", "rose"), ("🌻", "sunflower"),
        ("🌷", "tulip"), ("🌱", "seedling"), ("🌲", "evergreen tree"), ("🌴", "palm tree"), ("🍀", "four leaf clover luck"),
        ("🍁", "maple leaf"), ("🍂", "fallen leaf autumn"), ("🌍", "globe earth"), ("🌙", "crescent moon"),
        ("⭐", "star"), ("🌟", "glowing star"), ("☀️", "sun"), ("⛅", "sun behind cloud"), ("🌧️", "rain cloud"),
        ("⛄", "snowman"), ("❄️", "snowflake"), ("🌈", "rainbow"), ("🔥", "fire hot"), ("💧", "droplet"), ("🌊", "water wave"),
    ]),
    ("food", "Food & drink", &[
        ("🍎", "red apple"), ("🍊", "tangerine orange"), ("🍋", "lemon"), ("🍌", "banana"), ("🍉", "watermelon"),
        ("🍇", "grapes"), ("🍓", "strawberry"), ("🍒", "cherries"), ("🍑", "peach"), ("🥑", "avocado"),
        ("🥕", "carrot"), ("🌽", "corn"), ("🥐", "croissant"), ("🍞", "bread"), ("🧀", "cheese"), ("🍳", "cooking egg"),
        ("🥞", "pancakes"), ("🍔", "hamburger"), ("🍟", "french fries"), ("🍕", "pizza"), ("🌮", "taco"), ("🌯", "burrito"),
        ("🥗", "green salad"), ("🍝", "spaghetti"), ("🍜", "steaming bowl ramen"), ("🍣", "sushi"), ("🍦", "ice cream"),
        ("🍩", "doughnut"), ("🍪", "cookie"), ("🎂", "birthday cake"), ("🍰", "shortcake"), ("🧁", "cupcake"),
        ("🍫", "chocolate bar"), ("🍿", "popcorn"), ("☕", "hot beverage coffee"), ("🍵", "tea"), ("🥤", "cup with straw"),
        ("🍺", "beer"), ("🍻", "clinking beer mugs cheers"), ("🥂", "clinking glasses toast"), ("🍷", "wine glass"),
    ]),
    ("travel", "Travel & places", &[
        ("🚗", "car"), ("🚕", "taxi"), ("🚌", "bus"), ("🚆", "train"), ("🚇", "metro"), ("✈️", "airplane flight"),
        ("🛫", "airplane departure"), ("🛬", "airplane arrival"), ("🚀", "rocket launch"), ("🚲", "bicycle"),
        ("⛵", "sailboat"), ("🚢", "ship"), ("🏠", "house home"), ("🏢", "office building"), ("🏥", "hospital"),
        ("🏫", "school"), ("🏨", "hotel"), ("🏰", "castle"), ("🗽", "statue of liberty"), ("🗼", "tower"),
        ("⛰️", "mountain"), ("🏖️", "beach"), ("🏕️", "camping"), ("🌆", "cityscape"), ("🗺️", "world map"),
        ("🧭", "compass"), ("⏰", "alarm clock"), ("⌛", "hourglass"), ("📍", "round pushpin location"),
    ]),
    ("activities", "Activities", &[
        ("🎉", "party popper celebrate"), ("🎊", "confetti ball"), ("🎈", "balloon"), ("🎁", "gift present"),
        ("🎄", "christmas tree"), ("🎃", "jack-o-lantern"), ("🏆", "trophy win"), ("🥇", "first place medal"),
        ("🏅", "sports medal"), ("⚽", "soccer ball"), ("🏀", "basketball"), ("🏈", "american football"),
        ("⚾", "baseball"), ("🎾", "tennis"), ("🏐", "volleyball"), ("🏓", "ping pong"), ("⛳", "golf"),
        ("🎯", "direct hit target"), ("🎮", "video game"), ("🎲", "game die"), ("🧩", "puzzle piece"),
        ("🎨", "artist palette"), ("🎬", "clapper board"), ("🎤", "microphone"), ("🎧", "headphone"), ("🎵", "musical note"),
        ("🎸", "guitar"), ("🎹", "musical keyboard"), ("📚", "books"),
    ]),
    ("objects", "Objects", &[
        ("📧", "e-mail"), ("📨", "incoming envelope"), ("📩", "envelope with arrow"), ("📎", "paperclip attachment"),
        ("📌", "pushpin"), ("✏️", "pencil"), ("🖊️", "pen"), ("📝", "memo note"), ("📁", "file folder"), ("📂", "open folder"),
        ("📅", "calendar"), ("📆", "tear-off calendar"), ("📈", "chart increasing"), ("📉", "chart decreasing"),
        ("📊", "bar chart"), ("📋", "clipboard"), ("📞", "telephone receiver"), ("📱", "mobile phone"),
        ("💻", "laptop computer"), ("🖥️", "desktop computer"), ("⌨️", "keyboard"), ("🖨️", "printer"), ("💡", "light bulb idea"),
        ("🔋", "battery"), ("🔌", "electric plug"), ("🔒", "locked"), ("🔑", "key"), ("🔔", "bell notification"),
        ("📣", "megaphone"), ("💰", "money bag"), ("💳", "credit card"), ("🧾", "receipt"), ("☂️", "umbrella"),
        ("🕶️", "sunglasses"), ("👔", "necktie"), ("🛒", "shopping cart"), ("⏱️", "stopwatch"), ("🔍", "magnifying glass search"),
    ]),
    ("symbols", "Symbols", &[
        ("✅", "check mark button done"), ("☑️", "check box with check"), ("✔️", "check mark"), ("❌", "cross mark"),
        ("❎", "cross mark button"), ("➕", "plus"), ("➖", "minus"), ("❓", "question mark"), ("❗", "exclamation mark"),
        ("⚠️", "warning"), ("🚫", "prohibited"), ("⛔", "no entry"), ("♻️", "recycling"), ("🔴", "red circle"),
        ("🟠", "orange circle"), ("🟡", "yellow circle"), ("🟢", "green circle"), ("🔵", "blue circle"),
        ("🟣", "purple circle"), ("⚫", "black circle"), ("⚪", "white circle"), ("🔶", "large orange diamond"),
        ("🔷", "large blue diamond"), ("➡️", "right arrow"), ("⬅️", "left arrow"), ("⬆️", "up arrow"), ("⬇️", "down arrow"),
        ("🔁", "repeat"), ("🔄", "counterclockwise arrows"), ("🆗", "ok button"), ("🆕", "new button"), ("🆓", "free button"),
        ("ℹ️", "information"), ("©️", "copyright"), ("®️", "registered"), ("™️", "trade mark"), ("💲", "heavy dollar sign"),
        ("✨", "sparkles"), ("⚡", "high voltage lightning"),
    ]),
];

/// Emoji whose name contains every word of `query`, in category order.
pub fn search(query: &str) -> Vec<(&'static str, &'static str)> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    CATEGORIES
        .iter()
        .flat_map(|(_, _, list)| list.iter().copied())
        .filter(|(_, name)| words.iter().all(|w| name.contains(w.as_str())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_all_words() {
        assert!(search("thumbs").iter().any(|(e, _)| *e == "👍"));
        assert_eq!(search("thumbs up").len(), 1);
        assert!(search("zzzz nothing").is_empty());
        assert_eq!(search("").len(), CATEGORIES.iter().map(|c| c.2.len()).sum::<usize>());
    }
}
