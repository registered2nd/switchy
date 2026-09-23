//! Cost Calculator - computes API request cost
//!
//! Uses high-precision Decimal to avoid floating-point errors

use super::parser::TokenUsage;
use rust_decimal::Decimal;
use std::str::FromStr;

/// Cost breakdown
#[derive(Debug, Clone)]
pub struct CostBreakdown {
    pub input_cost: Decimal,
    pub output_cost: Decimal,
    pub cache_read_cost: Decimal,
    pub cache_creation_cost: Decimal,
    pub total_cost: Decimal,
}

/// Model pricing
#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub input_cost_per_million: Decimal,
    pub output_cost_per_million: Decimal,
    pub cache_read_cost_per_million: Decimal,
    pub cache_creation_cost_per_million: Decimal,
}

/// Cost calculator
pub struct CostCalculator;

impl CostCalculator {
    /// Calculates the request cost
    ///
    /// # Parameters
    /// - `usage`: token usage
    /// - `pricing`: model pricing
    /// - `cost_multiplier`: cost multiplier (set per provider)
    ///
    /// # Calculation
    /// - input_cost: (input_tokens - cache_read_tokens) × input price
    /// - cache_read_cost: cache_read_tokens × cache read price
    /// - so cached tokens are not billed twice
    /// - total_cost: sum of the component costs × multiplier (the multiplier applies only to the total)
    pub fn calculate(
        usage: &TokenUsage,
        pricing: &ModelPricing,
        cost_multiplier: Decimal,
    ) -> CostBreakdown {
        let million = Decimal::from(1_000_000);

        // Tokens billed at the input price (cache hits excluded)
        let billable_input_tokens = usage.input_tokens.saturating_sub(usage.cache_read_tokens);

        // Component base costs (before the multiplier)
        let input_cost =
            Decimal::from(billable_input_tokens) * pricing.input_cost_per_million / million;
        let output_cost =
            Decimal::from(usage.output_tokens) * pricing.output_cost_per_million / million;
        let cache_read_cost =
            Decimal::from(usage.cache_read_tokens) * pricing.cache_read_cost_per_million / million;
        let cache_creation_cost = Decimal::from(usage.cache_creation_tokens)
            * pricing.cache_creation_cost_per_million
            / million;

        // Total = sum of the component base costs × multiplier
        let base_total = input_cost + output_cost + cache_read_cost + cache_creation_cost;
        let total_cost = base_total * cost_multiplier;

        CostBreakdown {
            input_cost,
            output_cost,
            cache_read_cost,
            cache_creation_cost,
            total_cost,
        }
    }

    /// Calculates the cost, or returns None if the model is unknown
    pub fn try_calculate(
        usage: &TokenUsage,
        pricing: Option<&ModelPricing>,
        cost_multiplier: Decimal,
    ) -> Option<CostBreakdown> {
        pricing.map(|p| Self::calculate(usage, p, cost_multiplier))
    }
}

impl ModelPricing {
    /// Creates pricing from strings
    pub fn from_strings(
        input: &str,
        output: &str,
        cache_read: &str,
        cache_creation: &str,
    ) -> Result<Self, rust_decimal::Error> {
        Ok(Self {
            input_cost_per_million: Decimal::from_str(input)?,
            output_cost_per_million: Decimal::from_str(output)?,
            cache_read_cost_per_million: Decimal::from_str(cache_read)?,
            cache_creation_cost_per_million: Decimal::from_str(cache_creation)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_calculation() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_creation_tokens: 100,
            model: None,
        };

        let pricing = ModelPricing::from_strings("3.0", "15.0", "0.3", "3.75").unwrap();
        let multiplier = Decimal::from_str("1.0").unwrap();

        let cost = CostCalculator::calculate(&usage, &pricing, multiplier);

        // input: (1000 - 200) * 3.0 / 1M = 0.0024 (non-cached part only)
        assert_eq!(cost.input_cost, Decimal::from_str("0.0024").unwrap());
        // output: 500 * 15.0 / 1M = 0.0075
        assert_eq!(cost.output_cost, Decimal::from_str("0.0075").unwrap());
        // cache_read: 200 * 0.3 / 1M = 0.00006
        assert_eq!(cost.cache_read_cost, Decimal::from_str("0.00006").unwrap());
        // cache_creation: 100 * 3.75 / 1M = 0.000375
        assert_eq!(
            cost.cache_creation_cost,
            Decimal::from_str("0.000375").unwrap()
        );
        // total: 0.0024 + 0.0075 + 0.00006 + 0.000375 = 0.010335
        assert_eq!(cost.total_cost, Decimal::from_str("0.010335").unwrap());
    }

    #[test]
    fn test_cost_multiplier() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
        };

        let pricing = ModelPricing::from_strings("3.0", "15.0", "0", "0").unwrap();
        let multiplier = Decimal::from_str("1.5").unwrap();

        let cost = CostCalculator::calculate(&usage, &pricing, multiplier);

        // input_cost: base price (no multiplier) = 1000 * 3.0 / 1M = 0.003
        assert_eq!(cost.input_cost, Decimal::from_str("0.003").unwrap());
        // total_cost: base price × multiplier = 0.003 * 1.5 = 0.0045
        assert_eq!(cost.total_cost, Decimal::from_str("0.0045").unwrap());
    }

    #[test]
    fn test_unknown_model_handling() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
        };

        let multiplier = Decimal::from_str("1.0").unwrap();
        let cost = CostCalculator::try_calculate(&usage, None, multiplier);

        assert!(cost.is_none());
    }

    #[test]
    fn test_decimal_precision() {
        let usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 1,
            cache_creation_tokens: 1,
            model: None,
        };

        let pricing = ModelPricing::from_strings("0.075", "0.3", "0.01875", "0.075").unwrap();
        let multiplier = Decimal::from_str("1.0").unwrap();

        let cost = CostCalculator::calculate(&usage, &pricing, multiplier);

        // Check the high-precision result
        assert!(cost.total_cost > Decimal::ZERO);
        assert!(cost.total_cost.to_string().len() > 2); // decimal places are kept
    }
}
