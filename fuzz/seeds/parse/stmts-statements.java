class Statements {
    void m(int[] xs) throws Exception {
        ;
        int x = 1, y[] = {2};
        var list = new java.util.ArrayList<String>();
        java.util.List<String> names = list;
        x++;
        --x;
        x = y[0];
        this.m(xs);
        new Object();
        if (x > 0) x = 1; else if (x < 0) x = -1; else x = 0;
        while (x < 10) x++;
        do x--; while (x > 0);
        for (int i = 0, j = 0; i < j; i++, j--) ;
        for (x = 0; ; ) break;
        for (final int i : xs) continue;
        outer: for (;;) { break outer; }
        synchronized (this) { }
        assert x > 0 : "positive";
        class Local { }
        interface LocalInterface { }
        enum LocalEnum { A }
        record LocalRecord(int a) { }
        throw new Exception();
    }

    int r(int x) {
        return x;
    }
}
