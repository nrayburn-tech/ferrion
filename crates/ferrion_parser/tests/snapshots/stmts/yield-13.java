// release: 13
class Yield {
    void m(int x) {
        yield(1);
        yield = 1;
    }

    void yield(int x) { }

    int yield;
}
