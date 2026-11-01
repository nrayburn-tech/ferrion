package p;

import java.util.List;

public record Point<T>(@Deprecated int x, int y, List<T> tags) implements Comparable<Point<T>> {
    static final Point<String> ORIGIN = new Point<>(0, 0, List.of());

    public Point {
        if (x < 0) throw new IllegalArgumentException();
    }

    public int x() { return x; }

    public int compareTo(Point<T> other) { return 0; }

    record Pair(String first, String second) {
        Pair(String first) { this(first, first); }
        public String first() { return first; }
    }

    record Canonical(int[] values, String... rest) {
        Canonical(int values[], String[] rest) { this.values = values; this.rest = rest; }
    }

    record Empty() {}

    record Declared(int v) {
        public boolean equals(java.lang.Object o) { return false; }
        public int hashCode() { return v; }
        public String toString(int radix) { return ""; }
        public boolean equals(Declared other) { return true; }
    }
}
