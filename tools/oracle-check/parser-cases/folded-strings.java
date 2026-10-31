class F {
  Object a = "a" + "b" + "c";
  Object b = "a" + "b" + 1 + "c" + "d";
  Object c = 1 + 2 + "a" + "b";
  Object d = ("a" + "b") + "c";
  Object e = x == "a" + "b";
  Object f = "a" + "b" - 1;
  Object g = "a" + "b" + x * 2 + "c" + "d" == "e" + "f";
  Object h = x ? "a" + "b" : "c" + "d";
  Object i = "a" + "b" + "c" + "d" - "e" + "f" + "g";
  Object j = x + "b" + "c" + x;
  Object k = "a" + 'b' + "c";
  Object l = "a" + """
    b
    """ + f("c" + "d");
  Object m = "a" + "b" instanceof Object;
  static Object x;
  static native Object f(Object o);
}
