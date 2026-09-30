// Solve: which operands make rippled's real getRate == 651F8C1A7367E761?
#include <xrpl/basics/Number.h>
#include <boost/multiprecision/cpp_int.hpp>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <string>

using xrpl::Number;
using uint128_t = boost::multiprecision::uint128_t;
namespace xrpl {
[[noreturn]] void logicError(std::string const& s) noexcept { fprintf(stderr,"le:%s\n",s.c_str()); std::abort(); }
void logThrow(std::string const& s) { fprintf(stderr,"lt:%s\n",s.c_str()); }
}
static std::uint64_t muldiv(std::uint64_t a,std::uint64_t b,std::uint64_t d){return (std::uint64_t)((uint128_t(a)*uint128_t(b))/uint128_t(d));}

// getRate via issued normalize path (the correct path, matches rippled).
// num=in (native drops or iou), den=out (iou '1' => 1e15/-15)
static std::uint64_t getRate(std::uint64_t numVal,int numOff, bool numIntegral,
                             std::uint64_t denVal,int denOff, bool denIntegral) {
    const std::uint64_t T17=100000000000000000ull, MINV=1000000000000000ull;
    if(numIntegral) while(numVal<MINV){numVal*=10;--numOff;}
    if(denIntegral) while(denVal<MINV){denVal*=10;--denOff;}
    std::uint64_t rawM = muldiv(numVal,T17,denVal)+5;
    int rawO = numOff-denOff-17;
    // build Number(rawM * 10^rawO) exactly (rawM up to ~1e19, split)
    std::uint64_t hiPart = rawM/1000, loPart = rawM%1000;
    Number v = Number((long long)hiPart, rawO+3) + Number((long long)loPart, rawO);
    auto [im,ie] = v.normalizeToRange<1000000000000000LL,9999999999999999LL>();
    return ((std::uint64_t)(ie+100)<<56) | (std::uint64_t)im;
}

int main(){
    const std::uint64_t TARGET=0x651F8C1A7367E761ull;
    // Hyp A: as-stored: in=95669745624515984 XRP, out=1 IOU
    printf("A pays=95669745624515984 XRP / 1 IOU : %016llX\n",(unsigned long long)getRate(95669745624515984ull,0,true, 1000000000000000ull,-15,false));
    // Hyp B: maybe TakerGets '1' actually normalizes differently: try den mantissa 1e16 exp -16
    printf("B den=1e16/-16 : %016llX\n",(unsigned long long)getRate(95669745624515984ull,0,true, 10000000000000000ull,-16,false));
    // Reverse-engineer: target mantissa/exp
    std::uint64_t tm=TARGET&((1ull<<56)-1); int te=(int)(TARGET>>56)-100;
    printf("TARGET mantissa=%llu exp=%d  value~%llue%d\n",(unsigned long long)tm,te,(unsigned long long)tm,te);
    // If out=1 IOU (1e15/-15), then rate ~= in. So in ~ 8879769511257953e1 = 88797695112579530.
    // But as-stored TakerPays=95669745624515984. Ratio target/stored:
    printf("stored/target ratio = %.10f\n", 95669745624515984.0/88797695112579530.0);
    // Hyp C: in=88797695112579530 XRP / 1 IOU (what if pays was different?)
    printf("C pays=88797695112579530 XRP /1 IOU : %016llX\n",(unsigned long long)getRate(88797695112579530ull,0,true,1000000000000000ull,-15,false));
    return 0;
}
