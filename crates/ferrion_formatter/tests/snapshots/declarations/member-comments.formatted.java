/**
 * A doc comment before the package, then a blank line.
 */
package p;

/** A doc comment, then a blank line. */
class MemberComments {
  /** A doc comment, then a blank line. */
  int a;

  /* A block comment, then a blank line. */

  int b;
  // A line comment, then a blank line.

  int c;

  /** A doc comment, then a blank line before a comment. */

  // A line comment.
  void m() {}

  /** A doc comment on the field's line. */
  int d;

  /** A doc comment. */
  /* A block comment. */ void n() {}

  /* A block comment on the field's line. */ int e;

  /* A block comment on the method's line. */ @A
  void o() {}

  class Inner {
    /* A block comment on the line of the brace. */
    void f() {}
  }

  class Other { // A line comment on the line of the brace.
    void f() {}
  }

  class Empty {
    /* Alone in the body. */
  }

  @A // A line comment, then a blank line.
  void p() {}

  class EmptyWithLineComment { // Alone in the body.
  }

  @A
  /** A doc comment after an annotation. */
  void q() {}

  // A line comment before an annotation.
  @A // A line comment after it.
  // A line comment before the method.
  void r() {}

  void s(
      /** A doc comment before a parameter. */
      int a) {}

  class Last {
    int x; // A trailing comment, then blank lines before the brace.
  }
}
