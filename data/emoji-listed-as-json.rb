#!/usr/bin/env ruby
# encoding: utf-8

require 'oj'
require 'unicode/emoji'

class String
  def safe_toml_key
    safe_toml_key = self.dup
    safe_toml_key.gsub!(" ", "_")
    safe_toml_key.gsub!("&", "and")
    safe_toml_key.downcase!
    safe_toml_key
  end
end

emoji_list = {}
Unicode::Emoji.list.keys.each do |l|
    emoji_list[l.safe_toml_key] = {}
    emoji_list[l.safe_toml_key]['category'] = l
    Unicode::Emoji.list(l).keys.each do |subcategory|
        emoji_list[l.safe_toml_key][subcategory] = {}
        emoji_list[l.safe_toml_key][subcategory]['emojis'] = []
        Unicode::Emoji.list(l, subcategory).each do |emoji|
            emoji_list[l.safe_toml_key][subcategory]['emojis'] << emoji
        end
    end
end

File.open('./emoji.json', 'w+') do |f|
    f << Oj.dump(emoji_list)
end
