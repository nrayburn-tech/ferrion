// A small input for checking that the oracle runs.
package sample;

/** A record with a compact constructor. */
record Point(int x, int y) {
  Point {
    if (x < 0) throw new IllegalArgumentException(); /* trailing */
  }

  static int shift(int v) {
    return v >>> 2;
  }
}
