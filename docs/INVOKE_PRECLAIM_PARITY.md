# Full `invokePreclaim` parity plan

## Goal
Quaxar must use one transaction preclaim pipeline for both consensus `BuildLedger` and open-ledger/TxQ submission. It must match rippled's `invokePreclaim` behavior before any transaction can mutate a candidate view.

Reference path:
- `../rippled/src/libxrpl/tx/apply.cpp`
- `../rippled/src/libxrpl/tx/applySteps.cpp::invokePreclaim`
- `../rippled/src/libxrpl/tx/Transactor.cpp`
- `../rippled/src/libxrpl/tx/transactors/**`

Quaxar target paths:
- `xrpld/app/src/state/application_root.rs`
- `xrpld/app/src/state/transactor_dispatcher.rs`
- `xrpld/app/src/state/transactor_apply_bridge.rs`
- `xrpld/tx/src/**`

## Non-negotiable ordering
For every non-pseudo transaction, the shared dispatcher must execute against an immutable parent-backed `ReadView` in this order:

1. Stateless semantic preflight and amendment checks.
2. Cryptographic transaction signature validation.
3. `checkSeqProxy`: sequence/ticket validation.
4. `checkPriorTxAndLastLedger`: AccountTxnID, LastLedgerSequence, duplicate transaction check.
5. `checkPermission`: delegate authorization and transaction-specific permission rule.
6. Ledger-backed signer authorization: master-key-disabled, regular key, signer list, multisign quorum, and special signing overrides.
7. `calculateBaseFee` and `checkFee`.
8. Transaction-type-specific `preclaim`.
9. Only after a `tes*` or applicable `tec*` result may the apply/prelude path mutate a transaction sandbox.

The result must retain rippled's `Success` / `Fail` / `Retry` classification. A failed or panicked candidate must never install a ledger, publish a delta, mark a transaction committed, or stop the NetworkOps owner.

## Confirmed historical defects
| Defect | Root cause | Status |
| --- | --- | --- |
| Stale ancestor transaction replayed from ledger 106090758 into 106091161 | Consensus build bypassed sequence preclaim and directly rewrote `sfSequence` | Fixed locally; regression added |
| `networkops-strand` exited on SLE threading assertion | Candidate build panic was uncontained | Fixed locally at candidate boundary; must be reviewed with full dispatcher |
| Completion adoption stopped after strand exit | Completion persistence/checkAccept duplicated through strand and `LedgerDone` event consumer | Fixed locally; strand is the single owner |
| Candidate cache/delta effects before successful candidate materialization | TransactionMaster and publisher side effects occurred inside build loop | Deferred locally until candidate materialization |

## Current coverage inventory
Rippled macro-dispatches 75 transaction types. Quaxar routes those plus 5 Confidential-MPT types, for 80 dispatched types.

### Generic infrastructure that must be concrete
| Requirement | Existing primitive | Missing production adapter |
| --- | --- | --- |
| Semantic preflight | `tx::validate_sttx_transaction_preflight_with_rules` | Shared production dispatcher wiring |
| Sequence/ticket | `tx::run_transactor_check_seq_proxy` | Parent-backed STTx/ReadView adapter |
| Prior/last-ledger/duplicate | `tx::run_transactor_check_prior_tx_and_last_ledger` | Shared dispatcher wiring |
| Delegate permission | `tx::run_transactor_check_permission` | Delegate SLE reader and per-type permission dispatcher |
| Master/regular/multisign authorization | `tx::run_transactor_preclaim_check_sign` | AccountRoot/SignerList adapters and type overrides |
| Generic ordering | `tx::run_transactor_invoke_preclaim` | Concrete all-type invocation |
| Fee preclaim | transaction fee primitives | Parent-backed base-fee and fee-check adapter |
| Typed tail | scattered per-family fact helpers | One complete all-TxType typed preclaim dispatcher |

### Transaction-family matrix
Every routed type must be explicitly covered; no default `tesSUCCESS` placeholder is allowed.

| Family | Examples requiring typed parity | Current state |
| --- | --- | --- |
| System | Amendment, Fee, UNLModify, TicketCreate, Batch, LedgerStateFix | Batch has special preclaim; remaining system paths need matrix confirmation |
| Account | AccountSet, AccountDelete, SetRegularKey, DepositPreauth, CheckCreate/Cash/Cancel | Permission and typed facts incomplete |
| Payment | Payment, PaymentChannelCreate/Fund/Claim, EscrowCreate/Finish/Cancel, CheckCash | Permission and typed preclaim must be dispatched before apply |
| DEX/AMM | OfferCreate/Cancel, AMMCreate/Deposit/Withdraw/Vote/Bid/Delete/Clawback | Several helpers are placeholders or apply-adjacent |
| Trust/token | TrustSet, IssuedCurrency/Clawback, MPToken*, MPT*, Confidential MPT* | MPTokenIssuanceSet is partial; remaining paths need concrete facts |
| NFT | NFTokenMint/Burn/CreateOffer/AcceptOffer/CancelOffer/Modify | Typed tails must be read-only and explicit |
| Bridge/XChain | Bridge create/modify, XChain claim/commit/claim-ID/attestations | Missing typed preclaim fact builders |
| Permission/delegate/domain | DelegateSet, PermissionedDomainSet/Delete, DIDSet/Delete, Credential* | Delegate and typed permission paths missing |
| Oracle | OracleSet/Delete | OracleSet has apply-adjacent preclaim; move behind shared dispatcher |
| Vault/lending | Vault*, Loan*, LoanBroker*, LoanPay/Manage | LoanManage partial; broad typed fact coverage missing |

## Independent audit blockers (2026-08-06)

The midway independent audit found that the current dispatcher is still incomplete and must not be treated as deployable parity:

- `typed_preclaim_ter` registers only OracleSet, MPTokenIssuanceSet, and LoanManage. The remaining 77 routed types currently take a permissive success branch.
- The system-transaction shortcut bypasses generic preclaim for non-Batch system transactions, including TicketCreate and LedgerStateFix.
- Type-specific permission overrides are unregistered for AccountSet, Payment, TrustSet, MPTokenIssuanceSet, and LoanSet signer override.
- Generic fee preclaim lacks multisign increments and intentionally bypasses sponsor and specialized fee owners.
- Missing/unregistered typed tails span Change/Ticket/LedgerStateFix, Account, Payment/Escrow/Check, DEX/AMM, Token/Confidential-MPT, NFT, XChain, Identity/Domain/Credential, Vault, and remaining Lending families.

Implementation rule: replace the typed dispatcher default-success branch only when every routed TxType has an explicit implementation or an auditable rippled-equivalent no-op preclaim. Add system-family dispatch before removing the system shortcut.

## Implementation order
1. Define an immutable `ConsensusPreclaimView` adapter over `ReadView` with AccountRoot, SignerList, delegate, ticket, transaction, fee, and amendment reads.
2. Implement generic STTx adapter traits used by `run_transactor_invoke_preclaim` and signer/permission utilities.
3. Implement one `AppInvokePreclaimDispatcher` with no permissive default branch.
4. Port every typed rippled preclaim into a read-only Quaxar fact builder and register it in the dispatcher.
5. Route consensus BuildLedger and TxQ submission through exactly that dispatcher.
6. Remove existing duplicate/partial preclaim gates once coverage is proven.
7. Add table-driven tests for every dispatched `TxType` and tests for sequence, signer, delegate, fee, and typed preclaim ordering.
8. Require an independent audit with no BLOCKER or MAJOR findings before deployment.

## Validation gates
- `cargo test -p app --lib`
- targeted transaction-family matrix tests
- `cargo check -p app`
- `git diff --check`
- independent source audit against `../rippled`
- fresh mainnet reset/redeploy only after explicit approval
