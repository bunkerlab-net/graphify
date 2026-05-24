@extends('layouts.app')

@section('content')
    @include('partials.header')
    @include('partials.nav')

    <livewire:user-profile :user="$user" />
    <livewire:dashboard.widget />

    <button wire:click="save">Save</button>
    <button wire:click='delete'>Delete</button>
@endsection
