record Empty() {}

record Point(int x, int y) {}

public record Pair<A, B>(@NonNull A first, B second) implements Comparable<Pair<A, B>> {
  static int count;

  public Pair {
    count++;
  }

  Pair(A first) {
    this(first, null);
  }
}

record Varargs(String... names) {}
