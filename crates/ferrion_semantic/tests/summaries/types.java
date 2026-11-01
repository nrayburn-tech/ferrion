package p;

import java.lang.annotation.*;

class Types<K, V extends java.util.@A Map<K, ? extends @A V>> {
    java.util.Map.@A Entry<K, V>[] entries;
    @A int @B [] @C [] annotated;
    java.util.List<? super Integer> lower;
    java.util.List<?> any;
    java.util.Map<String, java.util.List<java.util.Map<Integer, int[][]>>> nested;
    Outer<String>.Inner<Integer> inner;

    <T extends Object & Comparable<? super T> & java.io.Serializable> T bounded() { return null; }

    void annotatedVarargs(String @A ... values) {}

    void receiver(@A Types<K, V> this, int x) {}
}

@Target(ElementType.TYPE_USE) @interface A {}
@Target(ElementType.TYPE_USE) @interface B {}
@Target(ElementType.TYPE_USE) @interface C {}
