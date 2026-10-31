// release: 14
class Yield {
    int m(int x) {
        int y = switch (x) {
            case 1 -> {
                yield (1);
            }
            case 2 -> {
                yield -1;
            }
            default -> {
                yield ++x;
            }
        };
        this.yield = 1;
        yield: for (;;) break yield;
        return y;
    }

    int yield;
}
