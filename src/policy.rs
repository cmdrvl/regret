//! Policy gate expression parser and evaluator.
//!
//! Implements --fail-if gating per §4.5:
//! - Grammar: <expr> := <term> (("and" | "or") <term>)*
//! - Term: <metric> <op> <number>
//! - Metrics: regret_events, max_score, events_per_100_commits
//! - Exit 3 on violation, exit 2 on parse error

use anyhow::{anyhow, Result};

/// Available metrics for policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Metric {
    RegretEvents,
    MaxScore,
    EventsPer100Commits,
}

impl Metric {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "regret_events" => Some(Metric::RegretEvents),
            "max_score" => Some(Metric::MaxScore),
            "events_per_100_commits" => Some(Metric::EventsPer100Commits),
            _ => None,
        }
    }
}

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompareOp {
    Gt,  // >
    Gte, // >=
    Lt,  // <
    Lte, // <=
    Eq,  // ==
    Neq, // !=
}

impl CompareOp {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            ">" => Some(CompareOp::Gt),
            ">=" => Some(CompareOp::Gte),
            "<" => Some(CompareOp::Lt),
            "<=" => Some(CompareOp::Lte),
            "==" => Some(CompareOp::Eq),
            "!=" => Some(CompareOp::Neq),
            _ => None,
        }
    }

    fn evaluate(&self, left: f64, right: f64) -> bool {
        match self {
            CompareOp::Gt => left > right,
            CompareOp::Gte => left >= right,
            CompareOp::Lt => left < right,
            CompareOp::Lte => left <= right,
            CompareOp::Eq => (left - right).abs() < f64::EPSILON,
            CompareOp::Neq => (left - right).abs() >= f64::EPSILON,
        }
    }
}

/// Boolean operators.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoolOp {
    And,
    Or,
}

impl BoolOp {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "and" => Some(BoolOp::And),
            "or" => Some(BoolOp::Or),
            _ => None,
        }
    }
}

/// A comparison term: metric <op> number
#[derive(Debug, Clone)]
pub struct Term {
    pub metric: Metric,
    pub op: CompareOp,
    pub value: f64,
}

/// A parsed policy expression.
#[derive(Debug, Clone)]
pub struct Expression {
    pub first: Term,
    pub rest: Vec<(BoolOp, Term)>,
}

/// Metric values for evaluation.
#[derive(Debug, Clone, Default)]
pub struct MetricValues {
    pub regret_events: usize,
    pub max_score: i64,
    pub events_per_100_commits: f64,
}

impl Expression {
    /// Evaluate the expression against the given metric values.
    /// Uses short-circuit evaluation.
    pub fn evaluate(&self, values: &MetricValues) -> bool {
        let mut result = self.first.evaluate(values);

        for (op, term) in &self.rest {
            match op {
                BoolOp::And => {
                    if !result {
                        // Short-circuit: already false
                        return false;
                    }
                    result = term.evaluate(values);
                }
                BoolOp::Or => {
                    if result {
                        // Short-circuit: already true
                        return true;
                    }
                    result = term.evaluate(values);
                }
            }
        }

        result
    }
}

impl Term {
    fn evaluate(&self, values: &MetricValues) -> bool {
        let metric_value = match self.metric {
            Metric::RegretEvents => values.regret_events as f64,
            Metric::MaxScore => values.max_score as f64,
            Metric::EventsPer100Commits => values.events_per_100_commits,
        };
        self.op.evaluate(metric_value, self.value)
    }
}

/// Token types for lexing.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Metric(Metric),
    Op(CompareOp),
    BoolOp(BoolOp),
    Number(f64),
}

/// Tokenize the input expression.
fn tokenize(input: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }

        // Try to match operators first (>=, <=, ==, !=, >, <)
        if c == '>' || c == '<' || c == '=' || c == '!' {
            let first = chars.next().unwrap();
            if chars.peek() == Some(&'=') {
                chars.next();
                let op_str = format!("{}=", first);
                let op = CompareOp::from_str(&op_str)
                    .ok_or_else(|| anyhow!("Invalid operator: {}", op_str))?;
                tokens.push(Token::Op(op));
            } else if first == '>' || first == '<' {
                let op = CompareOp::from_str(&first.to_string())
                    .ok_or_else(|| anyhow!("Invalid operator: {}", first))?;
                tokens.push(Token::Op(op));
            } else {
                return Err(anyhow!("Unexpected character: {}", first));
            }
            continue;
        }

        // Try to match number (including negative and decimal)
        if c.is_ascii_digit()
            || (c == '-' && chars.clone().nth(1).is_some_and(|n| n.is_ascii_digit()))
        {
            let mut num_str = String::new();
            if c == '-' {
                num_str.push(chars.next().unwrap());
            }
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() || c == '.' {
                    num_str.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            let num: f64 = num_str
                .parse()
                .map_err(|_| anyhow!("Invalid number: {}", num_str))?;
            tokens.push(Token::Number(num));
            continue;
        }

        // Try to match identifier (metric or boolean operator)
        if c.is_alphabetic() || c == '_' {
            let mut ident = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '_' {
                    ident.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            // Check if it's a boolean operator
            if let Some(bool_op) = BoolOp::from_str(&ident) {
                tokens.push(Token::BoolOp(bool_op));
                continue;
            }

            // Check if it's a metric
            if let Some(metric) = Metric::from_str(&ident) {
                tokens.push(Token::Metric(metric));
                continue;
            }

            return Err(anyhow!("Unknown identifier: {}", ident));
        }

        return Err(anyhow!("Unexpected character: {}", c));
    }

    Ok(tokens)
}

/// Parse a --fail-if expression.
pub fn parse_expression(input: &str) -> Result<Expression> {
    let tokens = tokenize(input)?;

    if tokens.is_empty() {
        return Err(anyhow!("Empty expression"));
    }

    let mut iter = tokens.into_iter().peekable();

    // Parse first term
    let first = parse_term(&mut iter)?;

    // Parse rest: (bool_op term)*
    let mut rest = Vec::new();
    while iter.peek().is_some() {
        let bool_op = match iter.next() {
            Some(Token::BoolOp(op)) => op,
            Some(t) => return Err(anyhow!("Expected 'and' or 'or', got {:?}", t)),
            None => break,
        };
        let term = parse_term(&mut iter)?;
        rest.push((bool_op, term));
    }

    Ok(Expression { first, rest })
}

/// Parse a single term: metric op number
fn parse_term(iter: &mut std::iter::Peekable<std::vec::IntoIter<Token>>) -> Result<Term> {
    let metric = match iter.next() {
        Some(Token::Metric(m)) => m,
        Some(t) => return Err(anyhow!("Expected metric, got {:?}", t)),
        None => return Err(anyhow!("Expected metric, got end of input")),
    };

    let op = match iter.next() {
        Some(Token::Op(o)) => o,
        Some(t) => return Err(anyhow!("Expected comparison operator, got {:?}", t)),
        None => return Err(anyhow!("Expected comparison operator, got end of input")),
    };

    let value = match iter.next() {
        Some(Token::Number(n)) => n,
        Some(t) => return Err(anyhow!("Expected number, got {:?}", t)),
        None => return Err(anyhow!("Expected number, got end of input")),
    };

    Ok(Term { metric, op, value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_expression() {
        let expr = parse_expression("regret_events >= 1").unwrap();
        assert_eq!(expr.first.metric, Metric::RegretEvents);
        assert_eq!(expr.first.op, CompareOp::Gte);
        assert_eq!(expr.first.value, 1.0);
        assert!(expr.rest.is_empty());
    }

    #[test]
    fn test_and_expression() {
        let expr = parse_expression("regret_events >= 1 and max_score > 20").unwrap();
        assert_eq!(expr.first.metric, Metric::RegretEvents);
        assert_eq!(expr.rest.len(), 1);
        assert_eq!(expr.rest[0].0, BoolOp::And);
        assert_eq!(expr.rest[0].1.metric, Metric::MaxScore);
        assert_eq!(expr.rest[0].1.op, CompareOp::Gt);
        assert_eq!(expr.rest[0].1.value, 20.0);
    }

    #[test]
    fn test_or_expression() {
        let expr = parse_expression("regret_events >= 5 or max_score > 50").unwrap();
        assert_eq!(expr.rest.len(), 1);
        assert_eq!(expr.rest[0].0, BoolOp::Or);
    }

    #[test]
    fn test_evaluation_true() {
        let expr = parse_expression("regret_events >= 1").unwrap();
        let values = MetricValues {
            regret_events: 5,
            max_score: 10,
            events_per_100_commits: 2.5,
        };
        assert!(expr.evaluate(&values));
    }

    #[test]
    fn test_evaluation_false() {
        let expr = parse_expression("regret_events >= 10").unwrap();
        let values = MetricValues {
            regret_events: 5,
            max_score: 10,
            events_per_100_commits: 2.5,
        };
        assert!(!expr.evaluate(&values));
    }

    #[test]
    fn test_and_short_circuit() {
        // First term is false, so second shouldn't matter
        let expr = parse_expression("regret_events >= 10 and max_score > 100").unwrap();
        let values = MetricValues {
            regret_events: 5,
            max_score: 10,
            events_per_100_commits: 2.5,
        };
        assert!(!expr.evaluate(&values));
    }

    #[test]
    fn test_or_short_circuit() {
        // First term is true, so second shouldn't matter
        let expr = parse_expression("regret_events >= 1 or max_score > 100").unwrap();
        let values = MetricValues {
            regret_events: 5,
            max_score: 10,
            events_per_100_commits: 2.5,
        };
        assert!(expr.evaluate(&values));
    }

    #[test]
    fn test_events_per_100_commits() {
        let expr = parse_expression("events_per_100_commits > 5.0").unwrap();
        let values = MetricValues {
            regret_events: 5,
            max_score: 10,
            events_per_100_commits: 7.5,
        };
        assert!(expr.evaluate(&values));
    }

    #[test]
    fn test_all_operators() {
        let values = MetricValues {
            regret_events: 5,
            max_score: 10,
            events_per_100_commits: 2.5,
        };

        assert!(parse_expression("regret_events > 4")
            .unwrap()
            .evaluate(&values));
        assert!(parse_expression("regret_events >= 5")
            .unwrap()
            .evaluate(&values));
        assert!(parse_expression("regret_events < 6")
            .unwrap()
            .evaluate(&values));
        assert!(parse_expression("regret_events <= 5")
            .unwrap()
            .evaluate(&values));
        assert!(parse_expression("regret_events == 5")
            .unwrap()
            .evaluate(&values));
        assert!(parse_expression("regret_events != 6")
            .unwrap()
            .evaluate(&values));
    }

    #[test]
    fn test_invalid_expression() {
        assert!(parse_expression("").is_err());
        assert!(parse_expression("invalid_metric >= 1").is_err());
        assert!(parse_expression("regret_events").is_err());
        assert!(parse_expression("regret_events >=").is_err());
        assert!(parse_expression("regret_events >= abc").is_err());
    }

    #[test]
    fn test_case_insensitive_bool_ops() {
        let expr = parse_expression("regret_events >= 1 AND max_score > 0").unwrap();
        assert_eq!(expr.rest[0].0, BoolOp::And);

        let expr = parse_expression("regret_events >= 1 OR max_score > 0").unwrap();
        assert_eq!(expr.rest[0].0, BoolOp::Or);
    }
}
