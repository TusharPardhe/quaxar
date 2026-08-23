# Canonical OfferCreate FOK evidence

This directory preserves byte-for-byte raw JSON-RPC request and response bodies
captured from `https://s1.ripple.com:51234/` for the fee-claiming FOK incident.
It is evidence only, not a replay fixture: no complete parent NodeStore snapshot
was downloaded, so these files must not be treated as an executable ledger test.

Each `*.tx.response.json` contains the canonical transaction and metadata. Each
`*.parent-ledger.response.json` pins the validated parent ledger header. The
owner and issuer `account_info` responses preserve the TickSize and reserve
context; `account_lines` records the queried owner/issuer relationship.

| Transaction | Child ledger | Parent ledger / hash | Flags | Canonical result |
| --- | ---: | --- | ---: | --- |
| `04A3DA3B3691EB7C0563788C3EEDE97094BAECFC5B5DB9B97E42443F18E82F57` | 106131663 | 106131662 / `E5052C9FE1097DE127AD2013462A1FD3FD65EAA6D28EEC2DEF0EA2FA0A09FA8A` | `0x00040000` (`tfFillOrKill`) | `tecKILLED` |
| `03BC7564835D60F48003E4EF09F516DAF4CF1FD1B8561C0544500818B809BC7E` | 106131673 | 106131672 / `78E53C5E55CAFB0ADD2934C63655070F77CEAA89BA300755FD211EAB9697DADB` | `0x00040000` (`tfFillOrKill`) | `tecKILLED` |

The metadata for both transactions records fee and Ticket consumption only;
there is no created Offer node. The issuing AccountRoot reports `TickSize = 6`
for STX (`04A3…`) and `TickSize = 5` for CTF (`03BC…`), so both transactions
exercise OfferCreate tick-size rounding before the FOK outcome.

## Response integrity

```text
483525c924bd5b35599d3c8c5ab43e7e8399e3e0211e77ff82a032105d0829ea  03BC7564835D60F48003E4EF09F516DAF4CF1FD1B8561C0544500818B809BC7E.tx.response.json
ebe7a20cf1524abd51cd23f71dc3a96eb1180cef3688936c81e158c25a5f7cd6  03BC7564835D60F48003E4EF09F516DAF4CF1FD1B8561C0544500818B809BC7E.parent-ledger.response.json
1c27f7a730433ccf852e60e0533deb19fa4f77eb0b1b2907c21f443c25413bc9  04A3DA3B3691EB7C0563788C3EEDE97094BAECFC5B5DB9B97E42443F18E82F57.parent-ledger.response.json
f6ae812c21f8f2a9692578fd3c19303a3e3bc1ce948a757bc6451948463f7055  04A3DA3B3691EB7C0563788C3EEDE97094BAECFC5B5DB9B97E42443F18E82F57.tx.response.json
6ffcf1aa22b682f3f4812b091632c5bd7a7517d3bc5ef9c427dd1cb9e145e780  03BC-issuer-parent-106131672.account-info.response.json
d22ff676cc369b0a81e1af7cd2150c350b00dce1268450100df58bc89e47b76b  03BC-owner-parent-106131672.account-info.response.json
6df61082c5dfd51f53e29fffa4e21455e109691baa3f62829f9205b2c18184b5  03BC-owner-issuer-parent-106131672.account-lines.response.json
6189fb28f3b1c5cef76dfdbd29d7d1467c06aab88e8936ea374c56a84acd1240  04A3-issuer-parent-106131662.account-info.response.json
56185d5d230a6c25f474ba4c67b1eca7808197dc68997a1f79a2b2d25bdcd2d9  04A3-owner-parent-106131662.account-info.response.json
7ec9682371f602691bd83507f4c274b0ba9c7c1cc65be89d27fa2ca305a5e69a  04A3-owner-issuer-parent-106131662.account-lines.response.json
```

Upstream comparison: `rippled/src/libxrpl/tx/transactors/dex/OfferCreate.cpp`
uses `Quality{saTakerGets, saTakerPays}.round(uTickSize).rate()` before
crossing, returns `tecKILLED` for a residual FOK at lines 807–813, and returns
`tecKILLED` for an IOC that transferred no funds at lines 815–827.
`rippled/src/libxrpl/tx/Transactor.cpp` resets fee-claiming `tecKILLED`
transactions and retains only unfunded-offer cleanup at lines 1253–1337.
