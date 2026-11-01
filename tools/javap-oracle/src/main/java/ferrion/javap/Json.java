package ferrion.javap;

/**
 * A minimal JSON writer, so the harness has no dependencies. A lone surrogate is written as
 * U+FFFD: JSON can't carry one through a UTF-8 stream, and the Rust side prints lone surrogates
 * the same way.
 */
final class Json {
  private final StringBuilder out = new StringBuilder();
  private boolean needsComma;

  Json beginObject() {
    separate();
    out.append('{');
    needsComma = false;
    return this;
  }

  Json endObject() {
    out.append('}');
    needsComma = true;
    return this;
  }

  Json beginArray() {
    separate();
    out.append('[');
    needsComma = false;
    return this;
  }

  Json endArray() {
    out.append(']');
    needsComma = true;
    return this;
  }

  Json name(String name) {
    value(name);
    out.append(':');
    needsComma = false;
    return this;
  }

  Json value(long value) {
    separate();
    out.append(value);
    needsComma = true;
    return this;
  }

  Json value(boolean value) {
    separate();
    out.append(value);
    needsComma = true;
    return this;
  }

  Json value(String value) {
    separate();
    out.append('"');
    for (int i = 0; i < value.length(); i++) {
      char c = value.charAt(i);
      switch (c) {
        case '"':
          out.append("\\\"");
          break;
        case '\\':
          out.append("\\\\");
          break;
        case '\n':
          out.append("\\n");
          break;
        case '\r':
          out.append("\\r");
          break;
        case '\t':
          out.append("\\t");
          break;
        default:
          if (c < 0x20) {
            out.append(String.format("\\u%04x", (int) c));
          } else if (Character.isHighSurrogate(c)
              && i + 1 < value.length()
              && Character.isLowSurrogate(value.charAt(i + 1))) {
            out.append(c).append(value.charAt(++i));
          } else if (Character.isSurrogate(c)) {
            out.append((char) 0xfffd);
          } else {
            out.append(c);
          }
      }
    }
    out.append('"');
    needsComma = true;
    return this;
  }

  private void separate() {
    if (needsComma) {
      out.append(',');
    }
  }

  @Override
  public String toString() {
    return out.toString();
  }
}
