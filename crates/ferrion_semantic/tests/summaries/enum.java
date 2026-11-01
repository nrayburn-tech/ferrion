package p;

public enum Planet implements Comparable<Planet>, Runnable {
    @Deprecated MERCURY(1.0),
    VENUS(2.0) {
        @Override public void run() {}
    },
    EARTH(3.0);

    private final double mass;

    Planet(double mass) { this.mass = mass; }

    public void run() {}

    enum Empty {}
}
