//! Full Rust migration surface for the deterministic `TER` result-category
//! catalog from `xrpl/protocol/TER.h` and the reference implementation.
//!
//! This now carries the reference integer wrapper, the full code catalog, token and
//! human-text lookup tables, and the reverse token lookup used by current
//! callers.

macro_rules! ter_catalog {
    ($macro:ident) => {
        $macro!(TEL_LOCAL_ERROR, -399, "telLOCAL_ERROR", "Local failure.");
        $macro!(TEL_BAD_DOMAIN, -398, "telBAD_DOMAIN", "Domain too long.");
        $macro!(
            TEL_BAD_PATH_COUNT,
            -397,
            "telBAD_PATH_COUNT",
            "Malformed: Too many paths."
        );
        $macro!(
            TEL_BAD_PUBLIC_KEY,
            -396,
            "telBAD_PUBLIC_KEY",
            "Public key is not valid."
        );
        $macro!(
            TEL_FAILED_PROCESSING,
            -395,
            "telFAILED_PROCESSING",
            "Failed to correctly process transaction."
        );
        $macro!(TEL_INSUF_FEE_P, -394, "telINSUF_FEE_P", "Fee insufficient.");
        $macro!(
            TEL_NO_DST_PARTIAL,
            -393,
            "telNO_DST_PARTIAL",
            "Partial payment to create account not allowed."
        );
        $macro!(
            TEL_CAN_NOT_QUEUE,
            -392,
            "telCAN_NOT_QUEUE",
            "Can not queue at this time."
        );
        $macro!(
            TEL_CAN_NOT_QUEUE_BALANCE,
            -391,
            "telCAN_NOT_QUEUE_BALANCE",
            "Can not queue at this time: insufficient balance to pay all queued fees."
        );
        $macro!(
            TEL_CAN_NOT_QUEUE_BLOCKS,
            -390,
            "telCAN_NOT_QUEUE_BLOCKS",
            "Can not queue at this time: would block later queued transaction(s)."
        );
        $macro!(
            TEL_CAN_NOT_QUEUE_BLOCKED,
            -389,
            "telCAN_NOT_QUEUE_BLOCKED",
            "Can not queue at this time: blocking transaction in queue."
        );
        $macro!(
            TEL_CAN_NOT_QUEUE_FEE,
            -388,
            "telCAN_NOT_QUEUE_FEE",
            "Can not queue at this time: fee insufficient to replace queued transaction."
        );
        $macro!(
            TEL_CAN_NOT_QUEUE_FULL,
            -387,
            "telCAN_NOT_QUEUE_FULL",
            "Can not queue at this time: queue is full."
        );
        $macro!(
            TEL_WRONG_NETWORK,
            -386,
            "telWRONG_NETWORK",
            "Transaction specifies a Network ID that differs from that of the local node."
        );
        $macro!(
            TEL_REQUIRES_NETWORK_ID,
            -385,
            "telREQUIRES_NETWORK_ID",
            "Transactions submitted to this node/network must include a correct NetworkID field."
        );
        $macro!(
            TEL_NETWORK_ID_MAKES_TX_NON_CANONICAL,
            -384,
            "telNETWORK_ID_MAKES_TX_NON_CANONICAL",
            "Transactions submitted to this node/network must NOT include a NetworkID field."
        );
        $macro!(
            TEL_ENV_RPC_FAILED,
            -383,
            "telENV_RPC_FAILED",
            "Unit test RPC failure."
        );

        $macro!(
            TEM_MALFORMED,
            -299,
            "temMALFORMED",
            "Malformed transaction."
        );
        $macro!(
            TEM_BAD_AMOUNT,
            -298,
            "temBAD_AMOUNT",
            "Malformed: Bad amount."
        );
        $macro!(
            TEM_BAD_CURRENCY,
            -297,
            "temBAD_CURRENCY",
            "Malformed: Bad currency."
        );
        $macro!(
            TEM_BAD_EXPIRATION,
            -296,
            "temBAD_EXPIRATION",
            "Malformed: Bad expiration."
        );
        $macro!(
            TEM_BAD_FEE,
            -295,
            "temBAD_FEE",
            "Invalid fee, negative or not XRP."
        );
        $macro!(
            TEM_BAD_ISSUER,
            -294,
            "temBAD_ISSUER",
            "Malformed: Bad issuer."
        );
        $macro!(
            TEM_BAD_LIMIT,
            -293,
            "temBAD_LIMIT",
            "Limits must be non-negative."
        );
        $macro!(TEM_BAD_OFFER, -292, "temBAD_OFFER", "Malformed: Bad offer.");
        $macro!(TEM_BAD_PATH, -291, "temBAD_PATH", "Malformed: Bad path.");
        $macro!(
            TEM_BAD_PATH_LOOP,
            -290,
            "temBAD_PATH_LOOP",
            "Malformed: Loop in path."
        );
        $macro!(
            TEM_BAD_REGKEY,
            -289,
            "temBAD_REGKEY",
            "Malformed: Regular key cannot be same as master key."
        );
        $macro!(
            TEM_BAD_SEND_XRP_LIMIT,
            -288,
            "temBAD_SEND_XRP_LIMIT",
            "Malformed: Limit quality is not allowed for XRP to XRP."
        );
        $macro!(
            TEM_BAD_SEND_XRP_MAX,
            -287,
            "temBAD_SEND_XRP_MAX",
            "Malformed: Send max is not allowed for XRP to XRP."
        );
        $macro!(
            TEM_BAD_SEND_XRP_NO_DIRECT,
            -286,
            "temBAD_SEND_XRP_NO_DIRECT",
            "Malformed: No Ripple direct is not allowed for XRP to XRP."
        );
        $macro!(
            TEM_BAD_SEND_XRP_PARTIAL,
            -285,
            "temBAD_SEND_XRP_PARTIAL",
            "Malformed: Partial payment is not allowed for XRP to XRP."
        );
        $macro!(
            TEM_BAD_SEND_XRP_PATHS,
            -284,
            "temBAD_SEND_XRP_PATHS",
            "Malformed: Paths are not allowed for XRP to XRP."
        );
        $macro!(
            TEM_BAD_SEQUENCE,
            -283,
            "temBAD_SEQUENCE",
            "Malformed: Sequence is not in the past."
        );
        $macro!(
            TEM_BAD_SIGNATURE,
            -282,
            "temBAD_SIGNATURE",
            "Malformed: Bad signature."
        );
        $macro!(
            TEM_BAD_SRC_ACCOUNT,
            -281,
            "temBAD_SRC_ACCOUNT",
            "Malformed: Bad source account."
        );
        $macro!(
            TEM_BAD_TRANSFER_RATE,
            -280,
            "temBAD_TRANSFER_RATE",
            "Malformed: Transfer rate must be >= 1.0 and <= 2.0"
        );
        $macro!(
            TEM_DST_IS_SRC,
            -279,
            "temDST_IS_SRC",
            "Destination may not be source."
        );
        $macro!(
            TEM_DST_NEEDED,
            -278,
            "temDST_NEEDED",
            "Destination not specified."
        );
        $macro!(
            TEM_INVALID,
            -277,
            "temINVALID",
            "The transaction is ill-formed."
        );
        $macro!(
            TEM_INVALID_FLAG,
            -276,
            "temINVALID_FLAG",
            "The transaction has an invalid flag."
        );
        $macro!(
            TEM_REDUNDANT,
            -275,
            "temREDUNDANT",
            "The transaction is redundant."
        );
        $macro!(
            TEM_RIPPLE_EMPTY,
            -274,
            "temRIPPLE_EMPTY",
            "PathSet with no paths."
        );
        $macro!(
            TEM_DISABLED,
            -273,
            "temDISABLED",
            "The transaction requires logic that is currently disabled."
        );
        $macro!(
            TEM_BAD_SIGNER,
            -272,
            "temBAD_SIGNER",
            "Malformed: No signer may duplicate account or other signers."
        );
        $macro!(
            TEM_BAD_QUORUM,
            -271,
            "temBAD_QUORUM",
            "Malformed: Quorum is unreachable."
        );
        $macro!(
            TEM_BAD_WEIGHT,
            -270,
            "temBAD_WEIGHT",
            "Malformed: Weight must be a positive value."
        );
        $macro!(
            TEM_BAD_TICK_SIZE,
            -269,
            "temBAD_TICK_SIZE",
            "Malformed: Tick size out of range."
        );
        $macro!(
            TEM_INVALID_ACCOUNT_ID,
            -268,
            "temINVALID_ACCOUNT_ID",
            "Malformed: A field contains an invalid account ID."
        );
        $macro!(
            TEM_CANNOT_PREAUTH_SELF,
            -267,
            "temCANNOT_PREAUTH_SELF",
            "Malformed: An account may not preauthorize itself."
        );
        $macro!(
            TEM_INVALID_COUNT,
            -266,
            "temINVALID_COUNT",
            "Malformed: Count field outside valid range."
        );
        $macro!(
            TEM_UNCERTAIN,
            -265,
            "temUNCERTAIN",
            "In process of determining result. Never returned."
        );
        $macro!(
            TEM_UNKNOWN,
            -264,
            "temUNKNOWN",
            "The transaction requires logic that is not implemented yet."
        );
        $macro!(
            TEM_SEQ_AND_TICKET,
            -263,
            "temSEQ_AND_TICKET",
            "Transaction contains a TicketSequence and a non-zero Sequence."
        );
        $macro!(
            TEM_BAD_NFTOKEN_TRANSFER_FEE,
            -262,
            "temBAD_NFTOKEN_TRANSFER_FEE",
            "Malformed: The NFToken transfer fee must be between 1 and 5000, inclusive."
        );
        $macro!(
            TEM_BAD_AMM_TOKENS,
            -261,
            "temBAD_AMM_TOKENS",
            "Malformed: Invalid LPTokens."
        );
        $macro!(
            TEM_XCHAIN_EQUAL_DOOR_ACCOUNTS,
            -260,
            "temXCHAIN_EQUAL_DOOR_ACCOUNTS",
            "Malformed: Bridge must have unique door accounts."
        );
        $macro!(
            TEM_XCHAIN_BAD_PROOF,
            -259,
            "temXCHAIN_BAD_PROOF",
            "Malformed: Bad cross-chain claim proof."
        );
        $macro!(
            TEM_XCHAIN_BRIDGE_BAD_ISSUES,
            -258,
            "temXCHAIN_BRIDGE_BAD_ISSUES",
            "Malformed: Bad bridge issues."
        );
        $macro!(
            TEM_XCHAIN_BRIDGE_NONDOOR_OWNER,
            -257,
            "temXCHAIN_BRIDGE_NONDOOR_OWNER",
            "Malformed: Bridge owner must be one of the door accounts."
        );
        $macro!(
            TEM_XCHAIN_BRIDGE_BAD_MIN_ACCOUNT_CREATE_AMOUNT,
            -256,
            "temXCHAIN_BRIDGE_BAD_MIN_ACCOUNT_CREATE_AMOUNT",
            "Malformed: Bad min account create amount."
        );
        $macro!(
            TEM_XCHAIN_BRIDGE_BAD_REWARD_AMOUNT,
            -255,
            "temXCHAIN_BRIDGE_BAD_REWARD_AMOUNT",
            "Malformed: Bad reward amount."
        );
        $macro!(
            TEM_EMPTY_DID,
            -254,
            "temEMPTY_DID",
            "Malformed: No DID data provided."
        );
        $macro!(
            TEM_ARRAY_EMPTY,
            -253,
            "temARRAY_EMPTY",
            "Malformed: Array is empty."
        );
        $macro!(
            TEM_ARRAY_TOO_LARGE,
            -252,
            "temARRAY_TOO_LARGE",
            "Malformed: Array is too large."
        );
        $macro!(
            TEM_BAD_TRANSFER_FEE,
            -251,
            "temBAD_TRANSFER_FEE",
            "Malformed: Transfer fee is outside valid range."
        );
        $macro!(
            TEM_INVALID_INNER_BATCH,
            -250,
            "temINVALID_INNER_BATCH",
            "Malformed: Invalid inner batch transaction."
        );
        $macro!(
            TEM_BAD_CIPHERTEXT,
            -249,
            "temBAD_CIPHERTEXT",
            "Malformed: Invalid ciphertext format."
        );

        $macro!(TEF_FAILURE, -199, "tefFAILURE", "Failed to apply.");
        $macro!(
            TEF_ALREADY,
            -198,
            "tefALREADY",
            "The exact transaction was already in this ledger."
        );
        $macro!(
            TEF_BAD_ADD_AUTH,
            -197,
            "tefBAD_ADD_AUTH",
            "Not authorized to add account."
        );
        $macro!(
            TEF_BAD_AUTH,
            -196,
            "tefBAD_AUTH",
            "Transaction's public key is not authorized."
        );
        $macro!(
            TEF_BAD_LEDGER,
            -195,
            "tefBAD_LEDGER",
            "Ledger in unexpected state."
        );
        $macro!(
            TEF_CREATED,
            -194,
            "tefCREATED",
            "Can't add an already created account."
        );
        $macro!(
            TEF_EXCEPTION,
            -193,
            "tefEXCEPTION",
            "Unexpected program state."
        );
        $macro!(TEF_INTERNAL, -192, "tefINTERNAL", "Internal error.");
        $macro!(
            TEF_NO_AUTH_REQUIRED,
            -191,
            "tefNO_AUTH_REQUIRED",
            "Auth is not required."
        );
        $macro!(
            TEF_PAST_SEQ,
            -190,
            "tefPAST_SEQ",
            "This sequence number has already passed."
        );
        $macro!(
            TEF_WRONG_PRIOR,
            -189,
            "tefWRONG_PRIOR",
            "This previous transaction does not match."
        );
        $macro!(
            TEF_MASTER_DISABLED,
            -188,
            "tefMASTER_DISABLED",
            "Master key is disabled."
        );
        $macro!(
            TEF_MAX_LEDGER,
            -187,
            "tefMAX_LEDGER",
            "Ledger sequence too high."
        );
        $macro!(
            TEF_BAD_SIGNATURE,
            -186,
            "tefBAD_SIGNATURE",
            "A signature is provided for a non-signer."
        );
        $macro!(
            TEF_BAD_QUORUM,
            -185,
            "tefBAD_QUORUM",
            "Signatures provided do not meet the quorum."
        );
        $macro!(
            TEF_NOT_MULTI_SIGNING,
            -184,
            "tefNOT_MULTI_SIGNING",
            "Account has no appropriate list of multi-signers."
        );
        $macro!(
            TEF_BAD_AUTH_MASTER,
            -183,
            "tefBAD_AUTH_MASTER",
            "Auth for unclaimed account needs correct master key."
        );
        $macro!(
            TEF_INVARIANT_FAILED,
            -182,
            "tefINVARIANT_FAILED",
            "Fee claim violated invariants for the transaction."
        );
        $macro!(
            TEF_TOO_BIG,
            -181,
            "tefTOO_BIG",
            "Transaction affects too many items."
        );
        $macro!(
            TEF_NO_TICKET,
            -180,
            "tefNO_TICKET",
            "Ticket is not in ledger."
        );
        $macro!(
            TEF_NFTOKEN_IS_NOT_TRANSFERABLE,
            -179,
            "tefNFTOKEN_IS_NOT_TRANSFERABLE",
            "The specified NFToken is not transferable."
        );
        $macro!(
            TEF_INVALID_LEDGER_FIX_TYPE,
            -178,
            "tefINVALID_LEDGER_FIX_TYPE",
            "The LedgerFixType field has an invalid value."
        );
        $macro!(
            TEF_NO_DST_PARTIAL,
            -177,
            "tefNO_DST_PARTIAL",
            "Partial payment not allowed to create account."
        );
        $macro!(
            TEF_BAD_PATH_COUNT,
            -176,
            "tefBAD_PATH_COUNT",
            "Too many paths or too long a path."
        );

        $macro!(TER_RETRY, -99, "terRETRY", "Retry transaction.");
        $macro!(TER_FUNDS_SPENT, -98, "terFUNDS_SPENT", "DEPRECATED.");
        $macro!(
            TER_INSUF_FEE_B,
            -97,
            "terINSUF_FEE_B",
            "Account balance can't pay fee."
        );
        $macro!(
            TER_NO_ACCOUNT,
            -96,
            "terNO_ACCOUNT",
            "The source account does not exist."
        );
        $macro!(
            TER_NO_AUTH,
            -95,
            "terNO_AUTH",
            "Not authorized to hold IOUs."
        );
        $macro!(TER_NO_LINE, -94, "terNO_LINE", "No such line.");
        $macro!(TER_OWNERS, -93, "terOWNERS", "Non-zero owner count.");
        $macro!(
            TER_PRE_SEQ,
            -92,
            "terPRE_SEQ",
            "Missing/inapplicable prior transaction."
        );
        $macro!(TER_LAST, -91, "terLAST", "DEPRECATED.");
        $macro!(
            TER_NO_RIPPLE,
            -90,
            "terNO_RIPPLE",
            "Path does not permit rippling."
        );
        $macro!(
            TER_QUEUED,
            -89,
            "terQUEUED",
            "Held until escalated fee drops."
        );
        $macro!(
            TER_PRE_TICKET,
            -88,
            "terPRE_TICKET",
            "Ticket is not yet in ledger."
        );
        $macro!(
            TER_NO_AMM,
            -87,
            "terNO_AMM",
            "AMM doesn't exist for the asset pair."
        );
        $macro!(
            TER_ADDRESS_COLLISION,
            -86,
            "terADDRESS_COLLISION",
            "Failed to allocate an unique account address."
        );
        $macro!(
            TER_NO_DELEGATE_PERMISSION,
            -85,
            "terNO_DELEGATE_PERMISSION",
            "Delegated account lacks permission to perform this transaction."
        );
        $macro!(
            TER_NO_SPONSORSHIP,
            -84,
            "terNO_SPONSORSHIP",
            "No sponsorship found."
        );

        $macro!(
            TES_SUCCESS,
            0,
            "tesSUCCESS",
            "The transaction was applied. Only final in a validated ledger."
        );

        $macro!(
            TEC_CLAIM,
            100,
            "tecCLAIM",
            "Fee claimed. Sequence used. No action."
        );
        $macro!(
            TEC_PATH_PARTIAL,
            101,
            "tecPATH_PARTIAL",
            "Path could not send full amount."
        );
        $macro!(TEC_UNFUNDED_ADD, 102, "tecUNFUNDED_ADD", "DEPRECATED.");
        $macro!(
            TEC_UNFUNDED_OFFER,
            103,
            "tecUNFUNDED_OFFER",
            "Insufficient balance to fund created offer."
        );
        $macro!(
            TEC_UNFUNDED_PAYMENT,
            104,
            "tecUNFUNDED_PAYMENT",
            "Insufficient XRP balance to send."
        );
        $macro!(
            TEC_FAILED_PROCESSING,
            105,
            "tecFAILED_PROCESSING",
            "Failed to correctly process transaction."
        );
        $macro!(
            TEC_DIR_FULL,
            121,
            "tecDIR_FULL",
            "Can not add entry to full directory."
        );
        $macro!(
            TEC_INSUF_RESERVE_LINE,
            122,
            "tecINSUF_RESERVE_LINE",
            "Insufficient reserve to add trust line."
        );
        $macro!(
            TEC_INSUF_RESERVE_OFFER,
            123,
            "tecINSUF_RESERVE_OFFER",
            "Insufficient reserve to create offer."
        );
        $macro!(
            TEC_NO_DST,
            124,
            "tecNO_DST",
            "Destination does not exist. Send XRP to create it."
        );
        $macro!(
            TEC_NO_DST_INSUF_XRP,
            125,
            "tecNO_DST_INSUF_XRP",
            "Destination does not exist. Too little XRP sent to create it."
        );
        $macro!(
            TEC_NO_LINE_INSUF_RESERVE,
            126,
            "tecNO_LINE_INSUF_RESERVE",
            "No such line. Too little reserve to create it."
        );
        $macro!(
            TEC_NO_LINE_REDUNDANT,
            127,
            "tecNO_LINE_REDUNDANT",
            "Can't set non-existent line to default."
        );
        $macro!(
            TEC_PATH_DRY,
            128,
            "tecPATH_DRY",
            "Path could not send partial amount."
        );
        $macro!(
            TEC_UNFUNDED,
            129,
            "tecUNFUNDED",
            "Not enough XRP to satisfy the reserve requirement."
        );
        $macro!(
            TEC_NO_ALTERNATIVE_KEY,
            130,
            "tecNO_ALTERNATIVE_KEY",
            "The operation would remove the ability to sign transactions with the account."
        );
        $macro!(
            TEC_NO_REGULAR_KEY,
            131,
            "tecNO_REGULAR_KEY",
            "Regular key is not set."
        );
        $macro!(TEC_OWNERS, 132, "tecOWNERS", "Non-zero owner count.");
        $macro!(
            TEC_NO_ISSUER,
            133,
            "tecNO_ISSUER",
            "Issuer account does not exist."
        );
        $macro!(
            TEC_NO_AUTH,
            134,
            "tecNO_AUTH",
            "Not authorized to hold asset."
        );
        $macro!(TEC_NO_LINE, 135, "tecNO_LINE", "No such line.");
        $macro!(
            TEC_INSUFF_FEE,
            136,
            "tecINSUFF_FEE",
            "Insufficient balance to pay fee."
        );
        $macro!(TEC_FROZEN, 137, "tecFROZEN", "Asset is frozen.");
        $macro!(
            TEC_NO_TARGET,
            138,
            "tecNO_TARGET",
            "Target account does not exist."
        );
        $macro!(
            TEC_NO_PERMISSION,
            139,
            "tecNO_PERMISSION",
            "No permission to perform requested operation."
        );
        $macro!(TEC_NO_ENTRY, 140, "tecNO_ENTRY", "No matching entry found.");
        $macro!(
            TEC_INSUFFICIENT_RESERVE,
            141,
            "tecINSUFFICIENT_RESERVE",
            "Insufficient reserve to complete requested operation."
        );
        $macro!(
            TEC_NEED_MASTER_KEY,
            142,
            "tecNEED_MASTER_KEY",
            "The operation requires the use of the Master Key."
        );
        $macro!(
            TEC_DST_TAG_NEEDED,
            143,
            "tecDST_TAG_NEEDED",
            "A destination tag is required."
        );
        $macro!(
            TEC_INTERNAL,
            144,
            "tecINTERNAL",
            "An internal error has occurred during processing."
        );
        $macro!(
            TEC_OVERSIZE,
            145,
            "tecOVERSIZE",
            "Object exceeded serialization limits."
        );
        $macro!(
            TEC_CRYPTOCONDITION_ERROR,
            146,
            "tecCRYPTOCONDITION_ERROR",
            "Malformed, invalid, or mismatched conditional or fulfillment."
        );
        $macro!(
            TEC_INVARIANT_FAILED,
            147,
            "tecINVARIANT_FAILED",
            "One or more invariants for the transaction were not satisfied."
        );
        $macro!(TEC_EXPIRED, 148, "tecEXPIRED", "Expiration time is passed.");
        $macro!(
            TEC_DUPLICATE,
            149,
            "tecDUPLICATE",
            "Ledger object already exists."
        );
        $macro!(
            TEC_KILLED,
            150,
            "tecKILLED",
            "No funds transferred and no offer created."
        );
        $macro!(TEC_AMM_NOT_FOUND, 151, "tecAMM_NOT_FOUND", "AMM not found.");
        $macro!(
            TEC_HAS_OBLIGATIONS,
            152,
            "tecHAS_OBLIGATIONS",
            "The account cannot be deleted since it has obligations."
        );
        $macro!(
            TEC_TOO_SOON,
            152,
            "tecTOO_SOON",
            "It is too early to attempt the requested operation. Please wait."
        );
        $macro!(
            TEC_HOOK_REJECTED,
            153,
            "tecHOOK_REJECTED",
            "Hook rejected the transaction."
        );
        $macro!(
            TEC_MAX_SEQUENCE_REACHED,
            154,
            "tecMAX_SEQUENCE_REACHED",
            "The maximum sequence number was reached."
        );
        $macro!(
            TEC_NO_SUITABLE_NFTOKEN_PAGE,
            155,
            "tecNO_SUITABLE_NFTOKEN_PAGE",
            "A suitable NFToken page could not be located."
        );
        $macro!(
            TEC_NFTOKEN_BUY_SELL_MISMATCH,
            156,
            "tecNFTOKEN_BUY_SELL_MISMATCH",
            "The 'Buy' and 'Sell' NFToken offers are mismatched."
        );
        $macro!(
            TEC_NFTOKEN_OFFER_TYPE_MISMATCH,
            157,
            "tecNFTOKEN_OFFER_TYPE_MISMATCH",
            "The type of NFToken offer is incorrect."
        );
        $macro!(
            TEC_CANT_ACCEPT_OWN_NFTOKEN_OFFER,
            158,
            "tecCANT_ACCEPT_OWN_NFTOKEN_OFFER",
            "An NFToken offer cannot be claimed by its owner."
        );
        $macro!(
            TEC_INSUFFICIENT_FUNDS,
            159,
            "tecINSUFFICIENT_FUNDS",
            "Not enough funds available to complete requested transaction."
        );
        $macro!(
            TEC_OBJECT_NOT_FOUND,
            160,
            "tecOBJECT_NOT_FOUND",
            "A requested object could not be located."
        );
        $macro!(
            TEC_INSUFFICIENT_PAYMENT,
            161,
            "tecINSUFFICIENT_PAYMENT",
            "The payment is not sufficient."
        );
        $macro!(
            TEC_UNFUNDED_AMM,
            162,
            "tecUNFUNDED_AMM",
            "Insufficient balance to fund AMM."
        );
        $macro!(
            TEC_AMM_BALANCE,
            163,
            "tecAMM_BALANCE",
            "AMM has invalid balance."
        );
        $macro!(
            TEC_AMM_FAILED,
            164,
            "tecAMM_FAILED",
            "AMM transaction failed."
        );
        $macro!(
            TEC_AMM_INVALID_TOKENS,
            165,
            "tecAMM_INVALID_TOKENS",
            "AMM invalid LP tokens."
        );
        $macro!(TEC_AMM_EMPTY, 166, "tecAMM_EMPTY", "AMM is in empty state.");
        $macro!(
            TEC_AMM_NOT_EMPTY,
            167,
            "tecAMM_NOT_EMPTY",
            "AMM is not in empty state."
        );
        $macro!(
            TEC_AMM_ACCOUNT,
            168,
            "tecAMM_ACCOUNT",
            "This operation is not allowed on an AMM Account."
        );
        $macro!(
            TEC_INCOMPLETE,
            169,
            "tecINCOMPLETE",
            "Some work was completed, but more submissions required to finish."
        );
        $macro!(
            TEC_XCHAIN_BAD_TRANSFER_ISSUE,
            170,
            "tecXCHAIN_BAD_TRANSFER_ISSUE",
            "Bad xchain transfer issue."
        );
        $macro!(
            TEC_XCHAIN_NO_CLAIM_ID,
            171,
            "tecXCHAIN_NO_CLAIM_ID",
            "No such xchain claim id."
        );
        $macro!(
            TEC_XCHAIN_BAD_CLAIM_ID,
            172,
            "tecXCHAIN_BAD_CLAIM_ID",
            "Bad xchain claim id."
        );
        $macro!(
            TEC_XCHAIN_CLAIM_NO_QUORUM,
            173,
            "tecXCHAIN_CLAIM_NO_QUORUM",
            "Quorum was not reached on the xchain claim."
        );
        $macro!(
            TEC_XCHAIN_PROOF_UNKNOWN_KEY,
            174,
            "tecXCHAIN_PROOF_UNKNOWN_KEY",
            "Unknown key for the xchain proof."
        );
        $macro!(
            TEC_XCHAIN_CREATE_ACCOUNT_NONXRP_ISSUE,
            175,
            "tecXCHAIN_CREATE_ACCOUNT_NONXRP_ISSUE",
            "Only XRP may be used for xchain create account."
        );
        $macro!(
            TEC_XCHAIN_WRONG_CHAIN,
            176,
            "tecXCHAIN_WRONG_CHAIN",
            "XChain Transaction was submitted to the wrong chain."
        );
        $macro!(
            TEC_XCHAIN_REWARD_MISMATCH,
            177,
            "tecXCHAIN_REWARD_MISMATCH",
            "The reward amount must match the reward specified in the xchain bridge."
        );
        $macro!(
            TEC_XCHAIN_NO_SIGNERS_LIST,
            178,
            "tecXCHAIN_NO_SIGNERS_LIST",
            "The account did not have a signers list."
        );
        $macro!(
            TEC_XCHAIN_SENDING_ACCOUNT_MISMATCH,
            179,
            "tecXCHAIN_SENDING_ACCOUNT_MISMATCH",
            "The sending account did not match the expected sending account."
        );
        $macro!(
            TEC_XCHAIN_INSUFF_CREATE_AMOUNT,
            180,
            "tecXCHAIN_INSUFF_CREATE_AMOUNT",
            "Insufficient amount to create an account."
        );
        $macro!(
            TEC_XCHAIN_ACCOUNT_CREATE_PAST,
            181,
            "tecXCHAIN_ACCOUNT_CREATE_PAST",
            "The account create count has already passed."
        );
        $macro!(
            TEC_XCHAIN_ACCOUNT_CREATE_TOO_MANY,
            182,
            "tecXCHAIN_ACCOUNT_CREATE_TOO_MANY",
            "There are too many pending account create transactions to submit a new one."
        );
        $macro!(
            TEC_XCHAIN_PAYMENT_FAILED,
            183,
            "tecXCHAIN_PAYMENT_FAILED",
            "Failed to transfer funds in a xchain transaction."
        );
        $macro!(
            TEC_XCHAIN_SELF_COMMIT,
            184,
            "tecXCHAIN_SELF_COMMIT",
            "Account cannot commit funds to itself."
        );
        $macro!(
            TEC_XCHAIN_BAD_PUBLIC_KEY_ACCOUNT_PAIR,
            185,
            "tecXCHAIN_BAD_PUBLIC_KEY_ACCOUNT_PAIR",
            "Bad public key account pair in an xchain transaction."
        );
        $macro!(
            TEC_XCHAIN_CREATE_ACCOUNT_DISABLED,
            186,
            "tecXCHAIN_CREATE_ACCOUNT_DISABLED",
            "This bridge does not support account creation."
        );
        $macro!(
            TEC_EMPTY_DID,
            187,
            "tecEMPTY_DID",
            "The DID object did not have a URI or DIDDocument field."
        );
        $macro!(
            TEC_INVALID_UPDATE_TIME,
            188,
            "tecINVALID_UPDATE_TIME",
            "The Oracle object has invalid LastUpdateTime field."
        );
        $macro!(
            TEC_TOKEN_PAIR_NOT_FOUND,
            189,
            "tecTOKEN_PAIR_NOT_FOUND",
            "Token pair is not found in Oracle object."
        );
        $macro!(TEC_ARRAY_EMPTY, 190, "tecARRAY_EMPTY", "Array is empty.");
        $macro!(
            TEC_ARRAY_TOO_LARGE,
            191,
            "tecARRAY_TOO_LARGE",
            "Array is too large."
        );
        $macro!(TEC_LOCKED, 192, "tecLOCKED", "Fund is locked.");
        $macro!(
            TEC_BAD_CREDENTIALS,
            193,
            "tecBAD_CREDENTIALS",
            "Bad credentials."
        );
        $macro!(TEC_WRONG_ASSET, 194, "tecWRONG_ASSET", "Wrong asset given.");
        $macro!(
            TEC_LIMIT_EXCEEDED,
            195,
            "tecLIMIT_EXCEEDED",
            "Limit exceeded."
        );
        $macro!(
            TEC_PSEUDO_ACCOUNT,
            196,
            "tecPSEUDO_ACCOUNT",
            "This operation is not allowed against a pseudo-account."
        );
        $macro!(
            TEC_PRECISION_LOSS,
            197,
            "tecPRECISION_LOSS",
            "The amounts used by the transaction cannot interact."
        );
        $macro!(
            TEC_NO_DELEGATE_PERMISSION,
            198,
            "tecNO_DELEGATE_PERMISSION",
            "This operation is not allowed against a pseudo-account."
        );
        $macro!(
            TEC_NO_ISSUANCE,
            199,
            "tecNO_ISSUANCE",
            "The specified MPT issuance does not exist."
        );
        $macro!(
            TEC_BAD_PROOF,
            200,
            "tecBAD_PROOF",
            "Zero-knowledge proof verification failed."
        );
        $macro!(
            TEC_NO_SPONSOR_PERMISSION,
            201,
            "tecNO_SPONSOR_PERMISSION",
            "Sponsor does not permit this operation."
        );
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Ter(i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerCatalogEntry {
    pub code: Ter,
    pub token: &'static str,
    pub human: &'static str,
}

macro_rules! define_ter_constant {
    ($name:ident, $value:expr, $token:expr, $human:expr) => {
        pub const $name: Self = Self($value);
    };
}

impl Ter {
    ter_catalog!(define_ter_constant);

    pub const fn from_int(value: i32) -> Self {
        Self(value)
    }

    pub const fn to_int(self) -> i32 {
        self.0
    }
}

pub type NotTec = Ter;

pub const fn is_tes_success(code: Ter) -> bool {
    code.to_int() == Ter::TES_SUCCESS.to_int()
}

pub const fn is_tec_claim(code: Ter) -> bool {
    code.to_int() >= Ter::TEC_CLAIM.to_int()
}

pub const fn is_ter_retry(code: Ter) -> bool {
    code.to_int() >= Ter::TER_RETRY.to_int() && code.to_int() < Ter::TES_SUCCESS.to_int()
}

pub const fn is_tem_malformed(code: Ter) -> bool {
    code.to_int() >= Ter::TEM_MALFORMED.to_int() && code.to_int() < Ter::TEF_FAILURE.to_int()
}

pub const fn is_tef_failure(code: Ter) -> bool {
    code.to_int() >= Ter::TEF_FAILURE.to_int() && code.to_int() < Ter::TER_RETRY.to_int()
}

pub fn trans_token(code: Ter) -> &'static str {
    macro_rules! maybe_return_token {
        ($name:ident, $value:expr, $token:expr, $human:expr) => {
            if code == Ter::$name {
                return $token;
            }
        };
    }

    if matches!(
        code,
        Ter::TEC_HOOK_REJECTED | Ter::TEC_NO_DELEGATE_PERMISSION
    ) {
        return "-";
    }

    ter_catalog!(maybe_return_token);
    "-"
}

pub fn trans_human(code: Ter) -> &'static str {
    macro_rules! maybe_return_human {
        ($name:ident, $value:expr, $token:expr, $human:expr) => {
            if code == Ter::$name {
                return $human;
            }
        };
    }

    if matches!(
        code,
        Ter::TEC_HOOK_REJECTED | Ter::TEC_NO_DELEGATE_PERMISSION
    ) {
        return "-";
    }

    ter_catalog!(maybe_return_human);
    "-"
}

pub fn trans_code(token: &str) -> Option<Ter> {
    macro_rules! maybe_return_code {
        ($name:ident, $value:expr, $token:expr, $human:expr) => {
            if token == $token {
                return Some(Ter::$name);
            }
        };
    }

    if matches!(token, "tecHOOK_REJECTED" | "tecNO_DELEGATE_PERMISSION") {
        return None;
    }

    ter_catalog!(maybe_return_code);
    None
}

pub fn trans_results() -> &'static [TerCatalogEntry] {
    static RESULTS: std::sync::OnceLock<Vec<TerCatalogEntry>> = std::sync::OnceLock::new();
    RESULTS
        .get_or_init(|| {
            let mut results = Vec::new();

            macro_rules! push_entry {
                ($name:ident, $value:expr, $token:expr, $human:expr) => {
                    results.push(TerCatalogEntry {
                        code: Ter::$name,
                        token: $token,
                        human: $human,
                    });
                };
            }

            ter_catalog!(push_entry);
            results
        })
        .as_slice()
}

#[cfg(test)]
mod tests {
    use super::{
        NotTec, Ter, is_tec_claim, is_tef_failure, is_tem_malformed, is_ter_retry, is_tes_success,
        trans_code, trans_human, trans_results, trans_token,
    };

    #[test]
    fn catalog_matches_current_cpp_codes_tokens_and_texts() {
        assert_eq!(trans_token(Ter::TEL_CAN_NOT_QUEUE), "telCAN_NOT_QUEUE");
        assert_eq!(
            trans_human(Ter::TEL_CAN_NOT_QUEUE),
            "Can not queue at this time."
        );
        assert_eq!(trans_code("telCAN_NOT_QUEUE"), Some(Ter::TEL_CAN_NOT_QUEUE));

        assert_eq!(
            trans_token(Ter::TEM_BAD_TRANSFER_FEE),
            "temBAD_TRANSFER_FEE"
        );
        assert_eq!(
            trans_human(Ter::TEM_BAD_TRANSFER_FEE),
            "Malformed: Transfer fee is outside valid range."
        );
        assert_eq!(
            trans_code("temBAD_TRANSFER_FEE"),
            Some(Ter::TEM_BAD_TRANSFER_FEE)
        );

        assert_eq!(trans_token(Ter::TEF_BAD_ADD_AUTH), "tefBAD_ADD_AUTH");
        assert_eq!(
            trans_human(Ter::TEF_BAD_ADD_AUTH),
            "Not authorized to add account."
        );
        assert_eq!(trans_code("tefBAD_ADD_AUTH"), Some(Ter::TEF_BAD_ADD_AUTH));

        assert_eq!(
            trans_token(Ter::TER_NO_DELEGATE_PERMISSION),
            "terNO_DELEGATE_PERMISSION"
        );
        assert_eq!(
            trans_human(Ter::TER_NO_DELEGATE_PERMISSION),
            "Delegated account lacks permission to perform this transaction."
        );
        assert_eq!(
            trans_code("terNO_DELEGATE_PERMISSION"),
            Some(Ter::TER_NO_DELEGATE_PERMISSION)
        );

        assert_eq!(trans_token(Ter::TES_SUCCESS), "tesSUCCESS");
        assert_eq!(
            trans_human(Ter::TES_SUCCESS),
            "The transaction was applied. Only final in a validated ledger."
        );
        assert_eq!(trans_code("tesSUCCESS"), Some(Ter::TES_SUCCESS));

        assert_eq!(trans_token(Ter::TEC_PSEUDO_ACCOUNT), "tecPSEUDO_ACCOUNT");
        assert_eq!(
            trans_human(Ter::TEC_PSEUDO_ACCOUNT),
            "This operation is not allowed against a pseudo-account."
        );
        assert_eq!(
            trans_code("tecPSEUDO_ACCOUNT"),
            Some(Ter::TEC_PSEUDO_ACCOUNT)
        );

        assert_eq!(trans_token(Ter::TEC_HOOK_REJECTED), "-");
        assert_eq!(trans_human(Ter::TEC_HOOK_REJECTED), "-");
        assert_eq!(trans_code("tecHOOK_REJECTED"), None);
        assert_eq!(trans_token(Ter::TEC_NO_DELEGATE_PERMISSION), "-");
        assert_eq!(trans_human(Ter::TEC_NO_DELEGATE_PERMISSION), "-");
        assert_eq!(trans_code("tecNO_DELEGATE_PERMISSION"), None);
    }

    #[test]
    fn result_category_helpers_match_current_cpp_ranges() {
        assert!(is_tes_success(Ter::TES_SUCCESS));
        assert!(!is_tes_success(Ter::TEC_CLAIM));

        assert!(is_ter_retry(Ter::from_int(-90)));
        assert!(!is_ter_retry(Ter::TES_SUCCESS));

        assert!(is_tem_malformed(Ter::TEM_MALFORMED));
        assert!(is_tem_malformed(Ter::TEM_BAD_TRANSFER_FEE));
        assert!(is_tem_malformed(Ter::TEM_UNCERTAIN));
        assert!(!is_tem_malformed(Ter::TEF_FAILURE));

        assert!(is_tef_failure(Ter::from_int(-150)));
        assert!(is_tef_failure(Ter::TEF_EXCEPTION));
        assert!(!is_tef_failure(Ter::from_int(-90)));

        assert!(is_tec_claim(Ter::TEC_CLAIM));
        assert!(is_tec_claim(Ter::TEC_EXPIRED));
        assert!(!is_tec_claim(Ter::TES_SUCCESS));
    }

    #[test]
    fn nottec_alias_can_hold_success_and_non_tec_codes() {
        let success: NotTec = Ter::TES_SUCCESS;
        let retry: NotTec = Ter::TER_RETRY;

        assert_eq!(success, Ter::TES_SUCCESS);
        assert_eq!(retry, Ter::TER_RETRY);
    }

    #[test]
    fn unknown_codes_and_tokens_fall_back() {
        let unknown = Ter::from_int(12345);

        assert_eq!(trans_token(unknown), "-");
        assert_eq!(trans_human(unknown), "-");
        assert_eq!(trans_code("notARealToken"), None);
    }

    #[test]
    fn trans_results_exposes_the_catalog_for_server_definitions() {
        let results = trans_results();
        assert!(results.iter().any(|entry| entry.code == Ter::TES_SUCCESS));
        assert!(results.iter().any(|entry| entry.token == "tecDIR_FULL"));
        assert!(results.iter().any(|entry| entry.human == "Local failure."));
    }
}
