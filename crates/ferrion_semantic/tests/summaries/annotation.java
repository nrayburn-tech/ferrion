package p;

import java.lang.annotation.*;

@Retention(RetentionPolicy.RUNTIME)
@Target({ElementType.TYPE, ElementType.METHOD})
public @interface Marker {
    String value() default "x";
    int[] numbers() default {1, 2};
    Class<?> type();
    int LIMIT = 10;
    enum Level { LOW, HIGH }
}
