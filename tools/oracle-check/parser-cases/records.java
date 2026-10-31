// releases: 15 16
record R<T>(int x, @Deprecated String... ys) implements Comparable<R<T>> {
  R {
    if (x < 0) throw new IllegalArgumentException();
  }
  public int compareTo(R<T> o) { return 0; }
}
