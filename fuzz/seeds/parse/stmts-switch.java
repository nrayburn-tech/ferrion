class Switch {
    int m(int x, Object o) {
        switch (x) {
            case 1:
            case 2, 3:
                x++;
                break;
            default:
        }
        switch (x) {
            case 1 -> x++;
            case 2, 3 -> { }
            case 4 -> throw new IllegalStateException();
            default -> { }
        }
        int y = switch (x) {
            case 1 -> 10;
            case 2 -> {
                yield 20;
            }
            default -> throw new IllegalStateException();
        };
        return switch (o) {
            case null, default -> y;
        };
    }
}
