class Patterns {
    record Point(int x, int y) { }
    record Box(Object o) { }

    String m(Object o) {
        if (o instanceof final String s && s.isEmpty()) return s;
        if (o instanceof Point(var x, _)) return "point";
        if (o instanceof int i) return "int";
        return switch (o) {
            case String s -> s;
            case Integer i when i > 0 -> "positive";
            case Point(int x, int y) when (x == y) -> "diagonal";
            case Box(Point(var x, var y)) -> "boxed";
            case Box(_) -> "box";
            case int[] a -> "ints";
            case null, default -> "other";
        };
    }

    int constants(int x) {
        return switch (x) {
            case A -> 1;
            case B, C -> 2;
            case (1) -> 3;
            default -> 0;
        };
    }

    static final int A = 0, B = 1, C = 2;
}
