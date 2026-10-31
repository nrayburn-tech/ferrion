class P {
  Object o;
  Object a = o instanceof final P x && o instanceof R(var y, R(int z, var w));
  Object b = o instanceof R(_, _) ? 1 : 2;
  Object n = o instanceof R(var(var x, var y), var z);
  Object c = switch (o) {
    case R(_, var _) -> 1;
    case String _ -> 2;
    case Integer i when i > 0 -> 3;
    case null, default -> 4;
  };
  record R(Object x, Object y) {}
}
