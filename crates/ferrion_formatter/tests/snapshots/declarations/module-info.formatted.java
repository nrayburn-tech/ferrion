@Deprecated open module a.b {
  requires transitive static c.d;
  exports e.f to g.h, i.j;
  opens k;
  uses l.M;
  provides n.O with p.Q, r.S;
}
