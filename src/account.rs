//! Random account identity generation.

use rand::Rng;
use rand::seq::IndexedRandom;

const FIRST: &[&str] = &[
    "Carl", "Dave", "Ethan", "Grace", "Hannah", "Isaac", "Julia", "Kevin", "Laura", "Marcus",
    "Nina", "Oscar", "Paula", "Quentin", "Rachel", "Sam", "Tina", "Victor", "Wendy", "Xavier",
    "Yara", "Zach", "Alice", "Brian",
];
const LAST: &[&str] = &[
    "Smith", "Jones", "Brown", "Miller", "Davis", "Wilson", "Moore", "Taylor", "Anderson",
    "Thomas", "Jackson", "White", "Harris", "Martin", "Thompson", "Clark", "Lewis", "Walker",
    "Hall", "Young", "King", "Wright", "Scott", "Green",
];
const WORDS: &[&str] = &[
    "Blaster", "Rocket", "Falcon", "Comet", "Pixel", "Nimbus", "Vortex", "Cobalt", "Ember",
    "Zephyr", "Quasar", "Onyx", "Lumen", "Drift", "Cinder", "Harbor", "Summit", "Echo",
];

/// Environment variable holding the catch-all recipient domain.
pub const CATCHALL_DOMAIN_ENV: &str = "CATCHALL_DOMAIN";

/// A generated account identity.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    /// Catch-all address (never verified).
    pub email: String,
    /// Account password.
    pub password: String,
    /// Catch-all recipient domain the email is built from.
    pub domain: String,
}

impl Account {
    /// Generates a random identity with a random password.
    pub fn generate(domain: &str) -> Self {
        let mut rng = rand::rng();
        Self::with_password(
            format!(
                "{}{}{}",
                WORDS.choose(&mut rng).expect("non-empty"),
                rng.random_range(100..1000),
                rng.random_range(10..100)
            ),
            domain,
        )
    }

    /// Generates a random catch-all email using the supplied fixed password.
    pub fn with_password(password: String, domain: &str) -> Self {
        let domain = domain.to_owned();
        let email = Self::random_email(&domain);
        Self {
            email,
            password,
            domain,
        }
    }

    /// Replaces the email with a fresh random catch-all address.
    pub fn regenerate_email(&mut self) {
        self.email = Self::random_email(&self.domain);
    }

    fn random_email(domain: &str) -> String {
        let mut rng = rand::rng();
        format!(
            "{}{}{}@{}",
            FIRST.choose(&mut rng).expect("non-empty"),
            LAST.choose(&mut rng).expect("non-empty"),
            rng.random_range(1000..10000),
            domain
        )
    }
}
