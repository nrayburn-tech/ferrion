class Creation {
    Object a = new List<>[2];
    Object b = new <String> Thread[4];
    Object c = new A.B<>[1][1];
    Object d = new A<>.B();
    Object e = outer.new Inner.Deep();
    Object f = this.class;
    Object g = a().class;
    Object h = a.b.class;
    Object i = C.<T>e.iterator();
    Object j = C.<T>e[0];
    Object k = C.<T>e.<U>f();
    Object l = C.<T>this;
}
