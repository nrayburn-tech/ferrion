// A file comment.


/* Before the package, after a blank line. */
package p; // trailing the package

import a.B; /* trailing, then a line break */
import c.D;
// leading the class, on its own line
/** The class's doc comment. */
@Deprecated /* leading the modifiers */ public class A /* before the body */ {

  // At the start of the body, after a blank line.
  int a = /* inside */ 1;
  int b = 2; // trailing the field
  int c = // trailing the `=`
      3;

  void m(/* dangling */) {
    // only a comment
  }

  void n() { /* dangling in the block */ }

  void o() {
    f(a, /* leads b */ b);
    f(a /* trails a */, b);
    g();
    // after the last statement
  }

        /*
         * Starred, re-indented.
         */
  void p() {}

  /* Not starred:
         keeps its shape
           relative to the least indented line
     */
  int q;
}
// The end of the file.
