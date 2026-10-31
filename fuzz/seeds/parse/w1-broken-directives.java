module m {
  requires;
  requires a
  foo a;
  exports a to;
  provides b;
  exports c.*;
  requires static static d;
}
