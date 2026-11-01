import a.B;

/** The module. */
@Deprecated
@SuppressWarnings("x")
open module a.b {
  requires transitive static c.d;
  requires e; // trailing
  requires f;

  exports e.f to
      g.h,
      i.j;
  exports k.l;

  opens m.n to
      o.p;
  opens q;

  // A comment before a directive.
  uses r.S;
  uses T;

  provides u.V with
      w.X,
      y.Z;
  provides A with
      B;
}
