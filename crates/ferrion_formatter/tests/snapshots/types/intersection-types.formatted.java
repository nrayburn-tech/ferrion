// Intersection types in casts that fit on their line.
class IntersectionTypes {
  void m() {
    Object o = (A & B) x;
    Object p = (Runnable & java.io.Serializable) () -> {};
    Object q = (Comparable<String> & java.io.Serializable & Cloneable) r;
  }
}
