// A class with every kind of member, and things a summary leaves out.
package com.example.shapes;

import java.util.List;
import java.util.*;
import static java.lang.Math.max;
import static java.util.Collections.*;
import module java.base;

/** Documentation isn't kept. */
@Deprecated(since = "9") @SuppressWarnings({"unchecked", "rawtypes"})
public abstract sealed class Shape<T extends Comparable<? super T>, U> extends Base<T>
        implements Comparable<Shape<T, U>>, java.io.Serializable permits Circle, Square {
    public static final int SIDES = 4 * 2, CORNERS;
    private List<String> names = new ArrayList<>();
    protected int[] matrix[], plain;
    volatile transient long counter;

    static { System.out.println("initializers are bodies"); }

    { counter = 1; }

    protected Shape(int sides) throws IllegalArgumentException {
        class Local {}
        this.counter = sides;
    }

    public abstract double area();

    public <R extends Number & Comparable<R>> List<R> scaled(final @Deprecated R factor, String... labels)
            throws java.io.IOException, IllegalStateException {
        Runnable r = new Runnable() { public void run() {} };
        return null;
    }

    int legacy(int x[])[] { return null; }

    native void peek();

    synchronized strictfp void sync() {}

    static class Nested<V> {
        int depth;

        interface Deeper {
            void go();
        }
    }

    enum Color { RED, GREEN }
}

final class Circle extends Shape<String, Object> {
    Circle() { super(0); }
    public double area() { return 0; }
}

non-sealed class Square extends Shape<String, Object> {
    public double area() { return 1; }
}
