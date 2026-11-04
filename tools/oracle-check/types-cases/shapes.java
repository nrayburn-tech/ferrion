// What javac's trees have that Ferrion's don't, and the other way around: compared with
// shapes.jsonl, the output of `javac-oracle types --release 21` on this file (Temurin 27.0.1).
import java.util.List;

class Q {
    record Implicit(int a, List<String> b) {}

    record Compact(int a) {
        Compact {
            a = 1;
        }
    }

    record Explicit(int a) {
        Explicit(int a) {
            this.a = a;
        }
    }

    enum E {
        PLAIN,
        ARGUMENT(1),
        BOTH(2) {
            int f() {
                return 1;
            }
        },
        BODY {
            int f() {
                return 2;
            }
        };

        E() {}

        E(int i) {}

        int f() {
            return 0;
        }
    }

    abstract static class Base {
        Base(int x, String y) {}
    }

    interface C extends AutoCloseable {}

    static final String K = "k";
    Object created = new Base(1, "a") {};
    /* Before UTF-16 offsets: 😀 😀 é */ Object anonymous = new Object() {};

    int conditions(C c, C d, List<String> xs, Object o) throws Exception {
        if (xs == null) {}
        while (true) {
            break;
        }
        do {} while (false);
        switch (1) {
            case 1:
                break;
        }
        switch ((2)) {
            default:
        }
        int k = switch (1) {
            default -> 2;
        };
        synchronized (this) {}
        synchronized (o) {}
        try (c; d) {}
        try (c; d;) {}
        try (C e = c) {}
        for (int i = 0; i < 1; i++) {}
        assert (k > 0);
        return k;
    }

    void strings(List<String> xs, int n) {
        String a = "a" + "b" + "c";
        String b = "a" + "b" + xs + "c" + "d";
        String c = xs + "a" + "b";
        String d = "a" + K + "b" + "c";
        String e = "a" + 1 + "b";
        String f = 1 + 2 + "a" + "b" + 3;
        String g = ("a" + "b") + "c" + "d";
        String h = """
            t""" + "x";
        String i = "a" + 'b' + "c";
        boolean j = "a" + "b" == "c" + "d";
        boolean l = n > 1 ? "a" + "b" == "e" : ("c" + "d") == "f";
        String m = "é" + "😀" + "\uD83D";
    }

    void literals() {
        long a = -1L + -2 + -0 + -0x10 + -010 + -(5) + - 3 + -2147483648 + -9223372036854775808L;
        double b = -1.0 + -1f;
        int c = -1_000;
    }

    void arrays() {
        int[] a = {1, 2};
        int[][] b = {{1}, {}};
        int[] c = new int[] {1};
        int[][] d = new int[][] {{1}, {2, 3}};
        int[] e = new int[2];
    }

    void locals(Object o) {
        class Local {
            int f;
        }
        Local l = new Local();
        var v = l;
        for (Object x : new Object[0]) {}
        Runnable r = () -> {};
        try {
        } catch (RuntimeException z) {
        }
        boolean b = o instanceof String s;
    }
}
