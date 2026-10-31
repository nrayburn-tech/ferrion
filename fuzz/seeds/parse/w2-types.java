import java.util.*;

class Types<T extends Comparable<? super T> & java.io.Serializable, U> {
  java.util.@A Map.@B Entry<String, List<? extends Number>> entry;
  int @A [] @B [] matrix;
  List<List<List<String>>> nested;
  Map<String, int[]>[] maps;

  <V extends T> V m(List<? super V> list, String @A ... rest) throws Exception, Error {
    return null;
  }

  void n() {
    try {
    } catch (IllegalStateException | IllegalArgumentException e) {
    }
  }
}
