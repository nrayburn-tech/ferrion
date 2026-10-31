class Recovery {
    void statements(int a) {
        a + 1;
        else a++;
        case 1: a++;
        catch (Exception e) { }
        try { }
        if (a > 0) int b = 1;
        while (a > 0) class C { }
        for (a : list) { }
        try (open()) { }
        f() g();
    }

    void switches(int a) {
        switch (a) { a++; case 1 -> return; }
        switch (a) { case default -> { } case 1, default -> { } }
        switch (a) { default when a > 0 -> { } }
        switch (a) { case 1 f(); }
    }

    void unclosed() {
        if (a > 0) {
            a++;

    private int field;
}
