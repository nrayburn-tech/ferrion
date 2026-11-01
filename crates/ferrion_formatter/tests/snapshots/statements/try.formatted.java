// `try` with resources, `catch` with its parameters, and `finally`.
class Try {
  void m() {
    try {
      b();
    } catch (E e) {
      c();
    }
    try {
      b();
    } catch (E e) {
      c();
    } catch (F f) {
      d();
    } finally {
      e();
    }
    try {
      b();
    } finally {
      c();
    }
    try (R r = open()) {
      b();
    }
    try (R r = open();
        S s = open()) {
      b();
    }
    try (R r = open();
        S s = open(); ) {
      b();
    }
    try (r;
        this.s) {
      b();
    }
    try (@Ann R r = open()) {}
    try (var r = open()) {
    } catch (E e) {
    }
    try (BufferedReader readerNumberOne = new BufferedReader(new FileReader(pathNumberOneX));
        BufferedReader readerNumberTwo = new BufferedReader(new FileReader(pathTwo))) {
      b();
    }
    try {
      b();
    } catch (IllegalStateException | IllegalArgumentException e) {
      c();
    }
    try {
      b();
    } catch (final IllegalStateException | IllegalArgumentException | NullPointerException e) {
      c();
    }
    try {
      b();
    } catch (IllegalStateExceptionWithAVeryVeryVeryVeryVeryVeryLongNameThatGoesOnAndOn exception) {
      c();
    }
    try {
      a();
    } catch (E e) {
      b();
    } finally {
      c();
    }
  }
}
