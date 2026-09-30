// Ground-truth harness using rippled's REAL Number arithmetic.
// Goal: what does static_cast<int64_t>(Number(95669745624515984.05)) give,
// i.e. the integral canonicalize of the divide result 9566974562451598405e-2.
#include <xrpl/basics/Number.h>
#include <boost/multiprecision/cpp_int.hpp>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <string>

using xrpl::Number;
using uint128_t = boost::multiprecision::uint128_t;

// Minimal stubs for the two contract.cpp symbols Number.cpp references.
namespace xrpl {
[[noreturn]] void logicError(std::string const& s) noexcept { fprintf(stderr, "logicError: %s\n", s.c_str()); std::abort(); }
void logThrow(std::string const& s) { fprintf(stderr, "logThrow: %s\n", s.c_str()); }
}

static std::uint64_t muldiv(std::uint64_t a, std::uint64_t b, std::uint64_t d) {
    return static_cast<std::uint64_t>((uint128_t(a) * uint128_t(b)) / uint128_t(d));
}

int main() {
    const std::uint64_t kTenTO17 = 100000000000000000ull;
    std::uint64_t numVal = 95669745624515984ull; int numOff = 0;
    std::uint64_t denVal = 1000000000000000ull;  int denOff = -15;

    std::uint64_t rawMant = muldiv(numVal, kTenTO17, denVal) + 5;   // 9566974562451598405
    int rawOff = numOff - denOff - 17;                             // -2
    printf("raw divide: mantissa=%llu offset=%d\n", (unsigned long long)rawMant, rawOff);

    // The divide result value = 9566974562451598405 * 10^-2 = 95669745624515984.05
    // Build that as a Number using the PUBLIC normalized ctor. rawMant exceeds
    // int64, so split: 9566974562451598405 = 9566974562451598 * 1000 + 405,
    // value = (9566974562451598 * 10^-2)*1000-ish -- instead build via two adds
    // using exact Number arithmetic (each operand fits int64).
    Number hi = Number(9566974562451598LL, rawOff + 3);  // 9566974562451598 * 10^1 = 95669745624515980
    Number lo = Number(405LL, rawOff);                   // 405 * 10^-2 = 4.05
    Number v = hi + lo;                                  // 95669745624515984.05
    // static_cast<int64_t>(Number) is what XRPAmount{num} does (integral path).
    std::int64_t drops = static_cast<std::int64_t>(v);
    printf("static_cast<int64_t>(divideResult) = %lld\n", (long long)drops);

    std::uint64_t mant = static_cast<std::uint64_t>(drops);
    std::uint64_t rate_integral = (static_cast<std::uint64_t>(0 + 100) << 56) | mant;
    printf("IF integral path: getRate = %016llX\n", (unsigned long long)rate_integral);

    // Alternative: IF the result were treated as ISSUED, normalize to [1e15,1e16):
    auto [im, ie] = v.normalizeToRange<1000000000000000LL, 9999999999999999LL>();
    std::uint64_t rate_issued = (static_cast<std::uint64_t>(ie + 100) << 56) | static_cast<std::uint64_t>(im);
    printf("IF issued path:   mantissa=%lld exp=%d getRate=%016llX\n",
           (long long)im, ie, (unsigned long long)rate_issued);

    printf("quaxar             = 6521FD1CD85EC08E\n");
    printf("canonical(testnet) = 651F8C1A7367E761\n");
    return 0;
}
