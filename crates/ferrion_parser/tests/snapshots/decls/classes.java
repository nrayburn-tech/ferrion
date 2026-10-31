package p;

/** A class with every header clause. */
public abstract class A<T extends Number> extends B<T> implements C, D<T> {
}

final class E {}

class F<K, V> implements G {}
