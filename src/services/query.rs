use sqlx::postgres::PgArguments;
use sqlx::Arguments;

pub struct QueryArgs {
    args: PgArguments,
    count: usize,
}

impl QueryArgs {
    pub fn new() -> Self {
        Self {
            args: PgArguments::default(),
            count: 0,
        }
    }

    pub fn add_str(&mut self, val: String) -> String {
        self.count += 1;
        let _ = self.args.add(val);
        format!("${}", self.count)
    }

    pub fn add_i32(&mut self, val: i32) -> String {
        self.count += 1;
        let _ = self.args.add(val);
        format!("${}", self.count)
    }

    pub fn add_i64(&mut self, val: i64) -> String {
        self.count += 1;
        let _ = self.args.add(val);
        format!("${}", self.count)
    }

    pub fn add_f64(&mut self, val: f64) -> String {
        self.count += 1;
        let _ = self.args.add(val);
        format!("${}", self.count)
    }

    pub fn add_opt_str(&mut self, val: Option<String>) -> String {
        self.count += 1;
        let _ = self.args.add(val);
        format!("${}", self.count)
    }

    pub fn into_args(self) -> PgArguments {
        self.args
    }

    pub fn len(&self) -> usize {
        self.count
    }
}
