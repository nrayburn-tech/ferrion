class Patterns {
    void m(Object o, int i, boolean flag) {
        if (o instanceof final Integer) {}
        if (o instanceof volatile Integer n) {}
        if (o instanceof final R(int x)) {}
        switch (o) {
            case final R(int x) -> {}
            case (Integer n) when n > 0 -> {}
            case Object x when flag = false -> {}
            default -> {}
        }
        switch (i) {
            case 0 when flag -> {}
            case 1 = 1 -> {}
            default -> {}
        }
    }
}
