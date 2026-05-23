import 'package:flutter/material.dart';
import 'dart:async';

class Counter {
  int value = 0;

  void increment() {
    value += 1;
  }

  int read() => value;
}

abstract class Animal {
  String name;
  Animal(this.name);
  void speak();
}

class Dog extends Animal {
  Dog(String name) : super(name);

  @override
  void speak() {
    print('Woof');
  }
}

mixin Greeter {
  String greet() => 'Hello';
}

enum Status { active, inactive }

void main() {
  final c = Counter();
  c.increment();
  print(c.read());
}
