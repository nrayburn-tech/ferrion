package p;

public sealed interface Vehicle<W extends Wheel> extends Comparable<Vehicle<W>>, AutoCloseable
        permits Car, Truck {
    int WHEELS = 4;
    String NAME = "vehicle" + WHEELS;

    W wheel();

    default void honk() { System.out.println("honk"); }

    static Vehicle<Wheel> none() { return null; }

    private void helper() {}
}
