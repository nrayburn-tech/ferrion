// Binary operators: a run of one precedence breaks before every operator at once, +4 from the
// first operand, and a run of higher precedence in an operand is a group of its own.
class Binary {
  void m() {
    x = a+b*c/d%e-f<<g>>h>>>i;
    b = a<b&&c>d||e<=f&&g>=h||i==j&&k!=l;
    x = a&b|c^d;
    x = firstOperandValue + secondOperandValue * thirdOperandValue - fourthOperandValue + fifthOperandValue;
    x = firstOperandValueAAAAAAAA + secondOperandValueBBBBBBBB + thirdOperandValueCCCCCCC + fourthOperandValueDDDDDD + fifth;
    x = aaaaaaaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb * ccccccccccccccccccccccccc * dddddddddddddddddddddddddddddddddddddd;
    x = aaaaaaaaaaaaaaaaaaaaaaaaaaa * bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb + ccccccccccccccccccccccccc * dddddddddddddddddddddddddddddddddddddd;
    x = a + (bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb + ccccccccccccccccccccccccccccccccccccccccccc + d);
    x = !(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa && bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb);
    x = a << bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb | ccccccccccccccccccccccccccccccccccccccc >>> 2 | d;
    message = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" + variable + "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" + other;
    x = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbbbbbb(cccccccccccccccccccccccccccc) + 1;
    x = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb + ccc(ddddddddddddddddddddddddd, eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, ffffffffffffffffffffff);
    foo(firstOperandValueAAAAAAAA + secondOperandValueBBBBBBBB + thirdOperandValueCCCCCCC + fourthOperandValueDDDDDD);
    if (firstConditionIsTrueAAAAAAA && secondConditionIsTrueBBBBBBBBB || thirdConditionIsTrueCCCCCCC && fourthDDDD) {}
    x = a // trailing the first operand
        + b;
    x = ++a + --b - a++ - b-- + - -c + + +d - -e + ~f + (!g ? 1 : 0);
  }
}
