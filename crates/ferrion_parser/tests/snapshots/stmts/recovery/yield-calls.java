class YieldCalls {
    Object m(int x) {
        yield(7, 8);
        yield();
        yield(a < b, c);
        yield(f(a, b));
        return switch (x) {
            case 1 -> {
                yield (a, b) -> a;
            }
            default -> {
                yield (1);
            }
        };
    }
}
