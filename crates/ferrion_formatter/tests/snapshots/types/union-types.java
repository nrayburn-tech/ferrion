// Union types: on one line if they fit, otherwise one alternative per line at +4.
class UnionTypes {
  void m() {
    try {
      x();
    } catch (A|B e) {
      x();
    }
    try {
      x();
    } catch (final @X A | B | C e) {
      x();
    }
    try {
      x();
    } catch (FirstVeryLongExceptionName | SecondVeryLongExceptionName | ThirdVeryLongExcepti e) {
      x();
    }
    try {
      x();
    } catch (FirstVeryLongExceptionName | SecondVeryLongExceptionName | ThirdVeryLongExceptionNa e) {
      x();
    }
    try {
      x();
    } catch (A | B | C | D | E | F | G | H | I | J | K | L | M | N | O | P | Q | R | S | T | U | V | W | X e) {
      x();
    }
    try {
      x();
    } catch (com.example.some.very.lng.pkg.name.that.goes.FirstException | com.example.other.SecondException e) {
      x();
    }
    try {
      x();
    } catch (A /* c */ | /* d */ B e) {
      x();
    }
    try {
      x();
    } catch (A | // c
        B e) {
      x();
    }
  }
}
