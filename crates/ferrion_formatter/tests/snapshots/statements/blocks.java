// Blocks: blank lines at their ends and between statements, and empty blocks by what they
// belong to.
class Blocks {
  void blankLines() {

    int a = 1;


    int b = 2;
    a++; b++;

  }

  void emptyBlocksStayTogether() {
    while (a) {}
    while (a) { }
    do {} while (a);
    for (;;) {}
    for (String s : list) {}
    if (a) {}
    try (r) {}
    Runnable r = () -> {};
  }

  void emptyBlocksBreak() {
    {}
    l: {}
    if (a) {} else {}
    if (a) b(); else if (c) {}
    try {} catch (E e) {} finally {}
    try (r) {} finally {}
    synchronized (this) {}
    switch (a) {}
  }

  void blankLineAfterTheBraceKept() {
    if (a) {

      b();
    } else {

      c();
    }
    while (a) {

      b();
    }
    try {

      b();
    } catch (E e) {

      c();
    } finally {

      d();
    }
  }

  void blankLineAfterTheBraceRemoved() {
    {

      a();
    }
    synchronized (this) {

      a();
    }
    l: {

      a();
    }
  }

  static {}

  static {

    a();
  }

  void emptyStatements() { ; ; }
}
