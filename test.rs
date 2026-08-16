use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
struct User {
    id: u32,
    name: String,
    score: i32,
    active: bool,
}

impl User {
    fn new(id: u32, name: &str, score: i32) -> Self {
        Self {
            id,
            name: name.to_string(),
            score,
            active: true,
        }
    }

    fn add_score(&mut self, amount: i32) {
        let old_score = self.score;
        self.score += amount;

        println!(
            "{}: score {} -> {}",
            self.name, old_score, self.score
        );
    }
}

fn calculate_bonus(score: i32) -> i32 {
    let multiplier = if score >= 100 { 3 } else { 2 };
    let bonus = score * multiplier;

    bonus
}

fn process_user(user: &mut User) -> Result<i32, String> {
    let original_score = user.score;

    if !user.active {
        return Err(format!("{} is inactive", user.name));
    }

    let bonus = calculate_bonus(user.score);

    user.add_score(bonus);

    let final_score = user.score;

    println!(
        "processed {}: {} -> {}",
        user.name, original_score, final_score
    );

    Ok(final_score)
}

fn process_users(users: &mut Vec<User>) -> i32 {
    let mut total = 0;

    for (index, user) in users.iter_mut().enumerate() {
        println!("processing index {}", index);

        match process_user(user) {
            Ok(score) => {
                total += score;
            }

            Err(error) => {
                println!("error: {}", error);
            }
        }
    }

    total
}

fn recursive_test(depth: u32) -> u32 {
    if depth == 0 {
        let bottom = 1234;

        // ★ BREAKPOINT:
        // Call Stack 테스트하기 아주 좋은 위치
        println!("bottom reached: {}", bottom);

        return bottom;
    }

    let next_depth = depth - 1;
    let result = recursive_test(next_depth);

    result + depth
}

fn worker_thread(name: &'static str, count: u32) {
    for i in 0..count {
        let value = i * i;

        println!(
            "[{}] iteration={}, value={}",
            name, i, value
        );

        thread::sleep(Duration::from_millis(100));
    }
}

fn main() {
    println!("=== Debug Test Start ===");

    let mut users = vec![
        User::new(1, "Alice", 10),
        User::new(2, "Bob", 50),
        User::new(3, "Charlie", 120),
    ];

    users[1].active = false;

    // ★ BREAKPOINT 1
    // Variables / Vec / Struct 테스트
    println!("users created: {:?}", users);

    let total = process_users(&mut users);

    // ★ BREAKPOINT 2
    // Step Into / Step Over 후 값 변화 확인
    println!("total score: {5}", total);

    let recursive_result = recursive_test(4);

    // ★ BREAKPOINT 3
    // recursive_test의 Call Stack 확인
    println!("recursive result: {}", recursive_result);

    let worker_a = thread::spawn(|| {
        worker_thread("worker-A", 5);
    });

    let worker_b = thread::spawn(|| {
        worker_thread("worker-B", 5);
    });

    // ★ BREAKPOINT 4
    // Threads 패널 테스트
    println!("workers started");

    worker_a.join().unwrap();
    worker_b.join().unwrap();

    let numbers = vec![1, 2, 3, 4, 5];

    let doubled: Vec<i32> = numbers
        .iter()
        .map(|x| x * 2)
        .collect();

    // ★ BREAKPOINT 5
    // Vec 펼치기 / expression evaluate 테스트
    println!("doubled: {:?}", doubled);

    let optional_value: Option<i32> = Some(42);

    if let Some(value) = optional_value {
        let squared = value * value;

        // ★ BREAKPOINT 6
        // Locals / scope 테스트
        println!("squared: {}", squared);
    }

    println!("=== Debug Test End ===");
}
