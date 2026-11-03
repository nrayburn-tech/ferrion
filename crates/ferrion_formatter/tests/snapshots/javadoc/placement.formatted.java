/** File header javadoc. */
/** Before package doc. */
package p;

/** Before import x */
import java.util.List;

/** Class doc here. */
class A {
  /** Field doc here. */
  int f;

  void m() {
    /** In body doc. */
    int x = 1;
    /** Before stmt x */
    foo();
    int y =
        /** in expr x */
        2;
  }

  /** Dangling at end. */
}
/** After class x */
