//! the reference implementation parity — lending protocol math: amortization,
//! interest computation, payment decomposition, and loan state management.

use basics::number::{NumberParts, get_mantissa_scale};
use protocol::{Rules, STTx, Ter, get_field_by_symbol};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

type RuntimeNumber = NumberParts;

/// Seconds in a year (365.25 days).
pub const SECONDS_IN_YEAR: u64 = 31_557_600;

/// Tenth basis points type.
pub type TenthBips32 = u32;

fn num_zero() -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(0, 0, get_mantissa_scale())
        .expect("zero should be representable")
}

fn num_from_i64(v: i64) -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(v, 0, get_mantissa_scale())
        .expect("small integer should be representable")
}

fn num_one() -> RuntimeNumber {
    num_from_i64(1)
}

/// Loan payment decomposition.
#[derive(Debug, Clone, PartialEq)]
pub struct LoanPaymentParts {
    pub principal_paid: RuntimeNumber,
    pub interest_paid: RuntimeNumber,
    pub value_change: RuntimeNumber,
    pub fee_paid: RuntimeNumber,
}

impl Default for LoanPaymentParts {
    fn default() -> Self {
        Self {
            principal_paid: num_zero(),
            interest_paid: num_zero(),
            value_change: num_zero(),
            fee_paid: num_zero(),
        }
    }
}

impl std::ops::AddAssign for LoanPaymentParts {
    fn add_assign(&mut self, other: Self) {
        self.principal_paid = self.principal_paid + other.principal_paid;
        self.interest_paid = self.interest_paid + other.interest_paid;
        self.value_change = self.value_change + other.value_change;
        self.fee_paid = self.fee_paid + other.fee_paid;
    }
}

/// Loan state tracking.
#[derive(Debug, Clone, PartialEq)]
pub struct LoanState {
    pub principal: RuntimeNumber,
    pub interest: RuntimeNumber,
    pub management_fee: RuntimeNumber,
}

/// Loan state deltas.
#[derive(Debug, Clone)]
pub struct LoanStateDeltas {
    pub principal: RuntimeNumber,
    pub interest: RuntimeNumber,
    pub management_fee: RuntimeNumber,
}

impl LoanStateDeltas {
    pub fn non_negative(&mut self) {
        if self.principal < num_zero() {
            self.principal = num_zero();
        }
        if self.interest < num_zero() {
            self.interest = num_zero();
        }
        if self.management_fee < num_zero() {
            self.management_fee = num_zero();
        }
    }
}

/// Check lending protocol amendment dependencies.
///
pub fn check_lending_protocol_dependencies(rules: &Rules, tx: &STTx) -> bool {
    if !rules.enabled(&protocol::feature_single_asset_vault()) {
        return false;
    }
    if !rules.enabled(&protocol::feature_mp_tokens_v1()) {
        return false;
    }
    if tx.is_field_present(sf("sfDomainID"))
        && !rules.enabled(&protocol::feature_permissioned_domains())
    {
        return false;
    }
    true
}

/// Convert annualized interest rate to per-payment-period rate.
///
pub fn loan_periodic_rate(interest_rate: TenthBips32, payment_interval: u32) -> RuntimeNumber {
    let rate = num_from_i64(interest_rate as i64);
    let interval = num_from_i64(payment_interval as i64);
    let basis = num_from_i64(100_000);
    let year = num_from_i64(SECONDS_IN_YEAR as i64);
    (rate * interval) / (basis * year)
}

/// Compute (1 + r)^n - 1 using binomial expansion.
///
pub fn compute_power_minus_one(
    periodic_rate: RuntimeNumber,
    payments_remaining: u32,
) -> RuntimeNumber {
    if payments_remaining == 0 || periodic_rate == num_zero() {
        return num_zero();
    }

    let n = num_from_i64(payments_remaining as i64);
    let mut term = n * periodic_rate;
    let mut sum = term;

    for k in 1..payments_remaining {
        let n_minus_k = num_from_i64((payments_remaining - k) as i64);
        let k_plus_1 = num_from_i64((k + 1) as i64);
        term = term * periodic_rate * n_minus_k / k_plus_1;
        let next = sum + term;
        if next == sum {
            break;
        }
        sum = next;
    }
    sum
}

/// Hybrid evaluator of (1 + r)^n - 1.
///
pub fn compute_power_minus_one_hybrid(
    periodic_rate: RuntimeNumber,
    payments_remaining: u32,
) -> RuntimeNumber {
    if payments_remaining == 0 || periodic_rate == num_zero() {
        return num_zero();
    }

    let threshold = RuntimeNumber::try_from_external_parts(1, -9, get_mantissa_scale())
        .unwrap_or_else(|_| num_zero());
    let product = num_from_i64(payments_remaining as i64) * periodic_rate;
    if product >= threshold {
        return power(num_one() + periodic_rate, payments_remaining) - num_one();
    }

    compute_power_minus_one(periodic_rate, payments_remaining)
}

/// Compute the payment factor for standard amortization.
///
pub fn compute_payment_factor(
    _rules: &Rules,
    periodic_rate: &RuntimeNumber,
    payments_remaining: u32,
) -> RuntimeNumber {
    if payments_remaining == 0 {
        return num_zero();
    }

    if *periodic_rate == num_zero() {
        return num_one() / num_from_i64(payments_remaining as i64);
    }

    let pmo = compute_power_minus_one_hybrid(*periodic_rate, payments_remaining);
    (*periodic_rate * (num_one() + pmo)) / pmo
}

/// Compute the periodic payment amount for a loan.
///
pub fn loan_periodic_payment(
    rules: &Rules,
    principal: RuntimeNumber,
    periodic_rate: &RuntimeNumber,
    payments_remaining: u32,
) -> RuntimeNumber {
    let factor = compute_payment_factor(rules, periodic_rate, payments_remaining);
    principal * factor
}

/// Compute principal from a periodic payment amount.
///
pub fn loan_principal_from_periodic_payment(
    rules: &Rules,
    payment: RuntimeNumber,
    periodic_rate: &RuntimeNumber,
    payments_remaining: u32,
) -> RuntimeNumber {
    let factor = compute_payment_factor(rules, periodic_rate, payments_remaining);
    if factor == num_zero() {
        return num_zero();
    }
    payment / factor
}

/// Compute interest and fee parts.
///
pub fn compute_interest_and_fee_parts(
    outstanding_principal: RuntimeNumber,
    periodic_rate: &RuntimeNumber,
    management_fee_rate: &RuntimeNumber,
) -> (RuntimeNumber, RuntimeNumber) {
    let interest = outstanding_principal * *periodic_rate;
    let fee = outstanding_principal * *management_fee_rate;
    (interest, fee)
}

/// Compute late payment interest.
///
pub fn loan_late_payment_interest(
    outstanding_principal: RuntimeNumber,
    periodic_rate: &RuntimeNumber,
    periods_late: u32,
) -> RuntimeNumber {
    if periods_late == 0 {
        return num_zero();
    }
    let compound = compute_power_minus_one_hybrid(*periodic_rate, periods_late);
    outstanding_principal * compound
}

/// Compute accrued interest.
///
pub fn loan_accrued_interest(
    outstanding_principal: RuntimeNumber,
    periodic_rate: &RuntimeNumber,
    elapsed_seconds: u32,
    payment_interval: u32,
) -> RuntimeNumber {
    if payment_interval == 0 || elapsed_seconds == 0 {
        return num_zero();
    }
    let fraction = num_from_i64(elapsed_seconds as i64) / num_from_i64(payment_interval as i64);
    outstanding_principal * *periodic_rate * fraction
}

/// Check loan guards.
///
pub fn check_loan_guards(
    principal: RuntimeNumber,
    _interest_rate: TenthBips32,
    payment_interval: u32,
    payments_remaining: u32,
) -> Ter {
    if principal <= num_zero() {
        return Ter::TEM_MALFORMED;
    }
    if payment_interval == 0 {
        return Ter::TEM_MALFORMED;
    }
    if payments_remaining == 0 {
        return Ter::TEM_MALFORMED;
    }
    Ter::TES_SUCCESS
}

/// Payment components for a loan payment.
#[derive(Debug, Clone)]
pub struct PaymentComponents {
    pub principal_part: RuntimeNumber,
    pub interest_part: RuntimeNumber,
    pub fee_part: RuntimeNumber,
    pub late_interest_part: RuntimeNumber,
}

impl PaymentComponents {
    pub fn tracked_interest_part(&self) -> RuntimeNumber {
        self.interest_part + self.late_interest_part
    }
}

/// Compute payment components.
///
pub fn compute_payment_components(
    payment_amount: RuntimeNumber,
    _outstanding_principal: RuntimeNumber,
    accrued_interest: RuntimeNumber,
    accrued_fee: RuntimeNumber,
    late_interest: RuntimeNumber,
) -> PaymentComponents {
    let mut remaining = payment_amount;

    let late_interest_part = if remaining >= late_interest {
        remaining -= late_interest;
        late_interest
    } else {
        let part = remaining;
        remaining = num_zero();
        part
    };

    let interest_part = if remaining >= accrued_interest {
        remaining -= accrued_interest;
        accrued_interest
    } else {
        let part = remaining;
        remaining = num_zero();
        part
    };

    let fee_part = if remaining >= accrued_fee {
        remaining -= accrued_fee;
        accrued_fee
    } else {
        let part = remaining;
        remaining = num_zero();
        part
    };

    let principal_part = remaining;

    PaymentComponents {
        principal_part,
        interest_part,
        fee_part,
        late_interest_part,
    }
}

// --- Internal math ---

fn power(base: RuntimeNumber, exp: u32) -> RuntimeNumber {
    if exp == 0 {
        return num_one();
    }
    let mut result = num_one();
    let mut b = base;
    let mut e = exp;
    while e > 0 {
        if e & 1 == 1 {
            result *= b;
        }
        b = b * b;
        e >>= 1;
    }
    result
}

/// Check if a value is already rounded to the specified scale.
///
pub fn is_rounded(value: RuntimeNumber, scale: i32) -> bool {
    let down = RuntimeNumber::try_from_external_parts(
        value.mantissa as i64,
        value.exponent + scale,
        get_mantissa_scale(),
    );
    let up = RuntimeNumber::try_from_external_parts(
        value.mantissa as i64,
        value.exponent + scale,
        get_mantissa_scale(),
    );
    down == up
}

/// Compute the full payment amount (principal + all interest + fees).
///
pub fn compute_full_payment(
    outstanding_principal: RuntimeNumber,
    accrued_interest: RuntimeNumber,
    accrued_fee: RuntimeNumber,
    late_interest: RuntimeNumber,
) -> RuntimeNumber {
    outstanding_principal + accrued_interest + accrued_fee + late_interest
}

/// Compute full payment interest (compound interest for remaining periods).
///
pub fn compute_full_payment_interest(
    outstanding_principal: RuntimeNumber,
    periodic_rate: &RuntimeNumber,
    periods_remaining: u32,
) -> RuntimeNumber {
    if periods_remaining == 0 || *periodic_rate == num_zero() {
        return num_zero();
    }
    let compound = compute_power_minus_one_hybrid(*periodic_rate, periods_remaining);
    outstanding_principal * compound
}

/// Compute late payment amount.
///
pub fn compute_late_payment(
    outstanding_principal: RuntimeNumber,
    periodic_rate: &RuntimeNumber,
    periods_late: u32,
) -> RuntimeNumber {
    loan_late_payment_interest(outstanding_principal, periodic_rate, periods_late)
}

/// Compute management fee.
///
pub fn compute_management_fee(
    outstanding_principal: RuntimeNumber,
    management_fee_rate: &RuntimeNumber,
) -> RuntimeNumber {
    outstanding_principal * *management_fee_rate
}

/// Compute overpayment components.
///
pub fn compute_overpayment_components(
    payment_amount: RuntimeNumber,
    outstanding_principal: RuntimeNumber,
    accrued_interest: RuntimeNumber,
    accrued_fee: RuntimeNumber,
    late_interest: RuntimeNumber,
) -> PaymentComponents {
    // Same logic as regular payment components — pay in priority order
    compute_payment_components(
        payment_amount,
        outstanding_principal,
        accrued_interest,
        accrued_fee,
        late_interest,
    )
}

/// Construct a loan state from SLE fields.
///
pub fn construct_loan_state(
    principal: RuntimeNumber,
    interest: RuntimeNumber,
    management_fee: RuntimeNumber,
) -> LoanState {
    LoanState {
        principal,
        interest,
        management_fee,
    }
}

/// Construct a rounded loan state.
///
pub fn construct_rounded_loan_state(
    principal: RuntimeNumber,
    interest: RuntimeNumber,
    management_fee: RuntimeNumber,
) -> LoanState {
    let scale = get_mantissa_scale();
    LoanState {
        principal: principal.truncate(scale),
        interest: interest.truncate(scale),
        management_fee: management_fee.truncate(scale),
    }
}

/// Compute theoretical loan state after a payment.
///
pub fn compute_theoretical_loan_state(
    current: &LoanState,
    payment: &PaymentComponents,
) -> LoanState {
    LoanState {
        principal: current.principal - payment.principal_part,
        interest: current.interest - payment.interest_part - payment.late_interest_part,
        management_fee: current.management_fee - payment.fee_part,
    }
}

/// Compute loan properties (periodic payment, total interest, etc.).
///
pub fn compute_loan_properties(
    rules: &Rules,
    principal: RuntimeNumber,
    periodic_rate: &RuntimeNumber,
    payments_remaining: u32,
) -> (RuntimeNumber, RuntimeNumber) {
    let periodic_payment =
        loan_periodic_payment(rules, principal, periodic_rate, payments_remaining);
    let total_payment = periodic_payment * num_from_i64(payments_remaining as i64);
    let total_interest = total_payment - principal;
    (periodic_payment, total_interest)
}

/// Execute a loan payment (the main doPayment logic).
///
pub fn do_payment(
    payment_amount: RuntimeNumber,
    outstanding_principal: RuntimeNumber,
    accrued_interest: RuntimeNumber,
    accrued_fee: RuntimeNumber,
    late_interest: RuntimeNumber,
) -> LoanPaymentParts {
    let components = compute_payment_components(
        payment_amount,
        outstanding_principal,
        accrued_interest,
        accrued_fee,
        late_interest,
    );

    LoanPaymentParts {
        principal_paid: components.principal_part,
        interest_paid: components.interest_part + components.late_interest_part,
        value_change: num_zero(),
        fee_paid: components.fee_part,
    }
}

/// Execute a loan make-payment operation.
///
pub fn loan_make_payment(
    payment_amount: RuntimeNumber,
    outstanding_principal: RuntimeNumber,
    accrued_interest: RuntimeNumber,
    accrued_fee: RuntimeNumber,
    late_interest: RuntimeNumber,
) -> LoanPaymentParts {
    do_payment(
        payment_amount,
        outstanding_principal,
        accrued_interest,
        accrued_fee,
        late_interest,
    )
}

/// Try overpayment — pays off remaining balance.
///
pub fn try_overpayment(
    payment_amount: RuntimeNumber,
    outstanding_principal: RuntimeNumber,
    accrued_interest: RuntimeNumber,
    accrued_fee: RuntimeNumber,
    late_interest: RuntimeNumber,
) -> Option<LoanPaymentParts> {
    let full = compute_full_payment(
        outstanding_principal,
        accrued_interest,
        accrued_fee,
        late_interest,
    );
    if payment_amount < full {
        return None;
    }

    Some(LoanPaymentParts {
        principal_paid: outstanding_principal,
        interest_paid: accrued_interest + late_interest,
        value_change: num_zero(),
        fee_paid: accrued_fee,
    })
}

impl std::ops::Sub for LoanState {
    type Output = LoanStateDeltas;
    fn sub(self, rhs: Self) -> LoanStateDeltas {
        LoanStateDeltas {
            principal: self.principal - rhs.principal,
            interest: self.interest - rhs.interest,
            management_fee: self.management_fee - rhs.management_fee,
        }
    }
}

impl std::ops::Sub<LoanStateDeltas> for LoanState {
    type Output = LoanState;
    fn sub(self, rhs: LoanStateDeltas) -> LoanState {
        LoanState {
            principal: self.principal - rhs.principal,
            interest: self.interest - rhs.interest,
            management_fee: self.management_fee - rhs.management_fee,
        }
    }
}

impl std::ops::Add<LoanStateDeltas> for LoanState {
    type Output = LoanState;
    fn add(self, rhs: LoanStateDeltas) -> LoanState {
        LoanState {
            principal: self.principal + rhs.principal,
            interest: self.interest + rhs.interest,
            management_fee: self.management_fee + rhs.management_fee,
        }
    }
}

/// Execute an overpayment — pays off the full remaining balance.
///
pub fn do_overpayment(
    payment_amount: RuntimeNumber,
    outstanding_principal: RuntimeNumber,
    accrued_interest: RuntimeNumber,
    accrued_fee: RuntimeNumber,
    late_interest: RuntimeNumber,
) -> LoanPaymentParts {
    let full = compute_full_payment(
        outstanding_principal,
        accrued_interest,
        accrued_fee,
        late_interest,
    );

    // If payment covers everything, pay it all off
    if payment_amount >= full {
        return LoanPaymentParts {
            principal_paid: outstanding_principal,
            interest_paid: accrued_interest + late_interest,
            value_change: num_zero(),
            fee_paid: accrued_fee,
        };
    }

    // Otherwise treat as a regular payment
    do_payment(
        payment_amount,
        outstanding_principal,
        accrued_interest,
        accrued_fee,
        late_interest,
    )
}
